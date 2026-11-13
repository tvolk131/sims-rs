//! Original-file live session shared by headless runners and interactive clients.
//! Unsupported native services stop the session without fabricating completion.
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sims_content::{
    Index, character_appearance,
    character_loading::{self, Presentation, Reader},
    saves::House,
};
use sims_sim::{
    Simulation, Snapshot, StepStop, lot_runtime,
    pose::{animator::Settings, properties, refresh::Response, transform},
};
use sims_vm::{ObjectId, Program};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
pub mod application_parent;
pub mod outer_parent;
pub mod draw_parent;
pub mod window_parent;
pub mod camera_command;
pub mod character_properties;
pub mod character_visibility;
pub mod clock;
pub mod completed_actions;
pub mod completed_animations;
pub mod completed_animation_cache;
pub mod completed_appearances;
pub mod completed_application;
pub mod completed_profiler;
pub mod completed_assets;
pub mod completed_baseline;
pub mod completed_controls;
pub mod completed_elapsed;
pub mod completed_epoch;
pub mod completed_frontend;
pub mod completed_geometry;
pub mod completed_groups;
pub mod completed_invocations;
pub mod completed_routes;
pub mod completed_route_controls;
pub mod completed_route_execution;
pub mod completed_lot;
pub mod completed_labels;
pub mod completed_object_storage;
pub mod completed_ordinary;
pub mod completed_person_world;
pub mod completed_presentation;
pub mod completed_fields;
pub mod completed_prototypes;
pub mod completed_residents;
pub mod completed_replay;
pub mod completed_routing;
pub mod completed_simulation;
pub mod completed_skeletons;
pub mod completed_skin;
pub mod completed_slot_anchors;
pub mod completed_stock;
pub mod completed_supplemental;
pub mod completed_tool_payloads;
pub mod completed_tools;
pub mod completed_viewer;
pub mod completed_world;
pub mod completion;
pub mod controllers;
pub mod current_tool;
pub mod cursor;
pub mod ground;
pub mod interface;
pub mod live_menu;
pub mod managed;
pub mod move_tool;
pub mod neighborhood_input;
pub mod notifications;
pub mod object_budget;
pub mod object_input;
pub mod panel;
pub mod person_appearance;
pub mod person_publication;
pub mod postload;
pub mod presenter;
mod products;
pub mod property_refresh;
pub mod publication;
pub mod purchase_cursor;
pub mod retained_frontend;
pub mod retained_tool_inputs;
pub mod tool_selection;
pub mod transport;
pub mod ui_wait;
pub mod valuation;
pub mod window_update;

/// Wire-format marker, not a scheduler choice. Retains byte-identical native
/// checkpoints while explicitly rejecting old legacy-only saved sessions.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RegistryScheduler;
impl Serialize for RegistryScheduler {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_bool(true)
    }
}
impl<'de> Deserialize<'de> for RegistryScheduler {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        if bool::deserialize(deserializer)? {
            Ok(Self)
        } else {
            Err(serde::de::Error::custom(
                "legacy scheduler snapshots are unsupported; reload the original house",
            ))
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Options {
    /// Explicit observed initial-process input for a NEW CC1.91 process only.
    /// Serde-missing historical options preserve unknown instead of opting in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub initial_process_profile: Option<sims_content::autonomy_resources::InitialProfile>,
    /// Explicit application-owned inputs at415eff. None keeps the historical
    /// diagnostic profile; a capture can supply its actual mode/hardware tier.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub application_context: Option<sims_vm::loading::HouseContext>,
    /// Original checkpoint wire marker. There is only one gameplay scheduler.
    #[serde(rename = "native_scheduler")]
    pub scheduler: RegistryScheduler,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub native_load_completion: bool,
    /// Retain43fc94's actual post-visit tail for the application owner.
    /// The caller must complete it before supplying the later F6 broadcast.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub native_postload: bool,
    pub routing: sims_sim::lot_route::path::Policy,
    /// The process CRT stream is a host input, separate from saved lot RNGs.
    pub crt_seed: u32,
    /// Diagnostic pre-initializer profile for F6's missing-cache branch.
    /// Normal43fc94 loading creates indicators before F6; keep that default.
    /// A deferred session requires an explicitly owned F6 before it can tick.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub defer_indicators: bool,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            application_context: None,
            initial_process_profile: Some(sims_content::autonomy_resources::InitialProfile::CompleteCollection191ObservedInitial20261024),
            scheduler: RegistryScheduler,
            native_load_completion: false,
            native_postload: false,
            routing: Default::default(),
            crt_seed: 0x51a5cafe,
            defer_indicators: false,
        }
    }
}

/// Software presentation owns no persistent GPU models or focus previews.
/// Selection and visibility are supplied by the client; sound playback is pending.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct View {
    /// Portable source bindings for retained provider identities. Targets are
    /// canonical resources; mutable targets must come from the owned namespace.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource_aliases: Option<BTreeMap<String, String>>,
    /// Explicit retained native property-service inputs. Older authored views
    /// retain their presentation-supplied context; no native values are inferred.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub character_properties: Option<character_properties::Source>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub character_visibility: Option<character_visibility::Source>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub person_draw: Option<person_appearance::State>,
    /// Explicit original process observation, owned by managed saves.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub neighborhood_input: Option<neighborhood_input::Captured>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub neighborhood_replacement: Option<neighborhood_input::replacement::Replacement>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub valuation_catalog: Option<sims_content::house_valuation::Catalog>,
    ///Explicit process6e4f64 binding. Older sessions preserve unknown state.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub person_selection_override: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_tool: Option<current_tool::State>,
    /// Actual process table/CPState/Picker inputs. A constructed tool driver
    /// cannot silently replace these retained owners.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retained_tool_inputs: Option<retained_tool_inputs::State>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub publication: Option<publication::State>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_selection: Option<tool_selection::State>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub move_tool: Option<move_tool::State>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub move_button: Option<move_tool::button::State>,
    /// Native view+1bc object pointer, stored by4ac2f4. This is separate from
    /// Animator preview/focus references. Absence preserves historical saves.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub object_focus: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub purchase_cursor: Option<purchase_cursor::State>,
    ///46d91c's synchronous DA child, retained until the object caller returns.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub object_budget: Option<object_budget::State>,
    /// Retained native picker script ID; the object may since have been deleted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub picker_target: Option<ObjectId>,
    ///Picker+40, cleared by48c8e4 before resolving the selected person.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub picker_activation_word: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ground_input: Option<ground::State>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub object_input: Option<object_input::State>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub live_menu: Option<live_menu::State>,
    /// Synchronous shared portrait retrieval owned by the actual UI client.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ui_portrait_tick: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub control_panel: Option<panel::State>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interface_update: Option<interface::State>,
    /// Explicit53a01f logical window backend; historical saves stay unbound.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub family_window: Option<window_update::State>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transport: Option<transport::State>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clock: Option<clock::State>,
    /// Actual4d1901 parent of the retained elapsed call. Historical absence
    /// stays unknown; importing it never re-enters the application frame.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub application_parent: Option<application_parent::State>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ui_wait: Option<ui_wait::State>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub camera_ui: Option<sims_sim::lot_presenter::camera_command::Ui>,
    /// Actual persistent native publication inputs, not an invented active call.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frontend_context: Option<retained_frontend::Fields>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub camera_command: Option<camera_command::State>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loading: Option<completion::State>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub postload: Option<postload::State>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub awaiting_house_open: bool,
    pub selected: Option<ObjectId>,
    pub viewport: [i32; 4],
    /// Last explicit native camera input; headless sessions use the initialized view.
    #[serde(default)]
    pub camera: Option<sims_sim::pose::portrait::view::Input>,
    /// None requests updates for every actor, as in the headless replay.
    pub visible: Option<BTreeSet<ObjectId>>,
    /// Explicit F6 caller and its nested classification/indicator continuation.
    /// Older sessions omit it; loading does not silently dispatch a new F6.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub presenter: Option<presenter::State>,
    /// Shared original broadcast and its saved viewer callback. Explicitly
    /// installed by the host; older saves do not invent a startup registry.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notifications: Option<notifications::State>,
    /// Retained Interface4ce41b caller. Older saves stay explicitly unbound.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub property_refresh: Option<property_refresh::State>,
}
impl Default for View {
    fn default() -> Self {
        Self {
            resource_aliases: None,
            character_properties: None,
            character_visibility: None,
            person_draw: None,
            neighborhood_input: None,
            neighborhood_replacement: None,
            valuation_catalog: None,
            person_selection_override: None,
            publication: None,
            tool_selection: None,
            move_tool: None,
            move_button: None,
            purchase_cursor: None,
            current_tool: None,
            retained_tool_inputs: None,
            object_focus: None,
            object_budget: None,
            picker_target: None,
            picker_activation_word: None,
            ground_input: None,
            object_input: None,
            live_menu: None,
            ui_portrait_tick: None,
            control_panel: None,
            interface_update: None,
            family_window: None,
            transport: None,
            clock: None,
            application_parent: None,
            ui_wait: None,
            camera_ui: None,
            frontend_context: None,
            camera_command: None,
            loading: None,
            postload: None,
            awaiting_house_open: false,
            selected: None,
            viewport: [0, 0, 800, 600],
            camera: None,
            visible: None,
            presenter: None,
            notifications: None,
            property_refresh: None,
        }
    }
}
impl Presentation for View {
    fn context_at(
        &mut self,
        id: ObjectId,
        placement: &transform::Placement,
        appearance_mode: i32,
        world: &sims_vm::World,
    ) -> Result<properties::Context> {
        match self.character_properties {
            Some(source) => source.context_at(id, placement, appearance_mode, world),
            None => self.context(id, placement),
        }
    }
    fn context_in_world(
        &mut self,
        id: ObjectId,
        resident: &sims_sim::character::Resident,
        world: &sims_vm::World,
    ) -> Result<properties::Context> {
        match self.character_properties {
            Some(source) => source.context(id, resident, world),
            None => self.context(id, &resident.character.placement),
        }
    }
    fn retirement_references(
        &mut self,
        id: ObjectId,
    ) -> Result<character_loading::RetirementReferences> {
        Ok(character_loading::RetirementReferences {
            selected: self.selected == Some(id),
            focus: false,
            base_focus: false,
        })
    }
    fn retire(&mut self, id: ObjectId, event: character_loading::RetirementEvent) -> Result<()> {
        if matches!(event, character_loading::RetirementEvent::ClearSelected)
            && self.selected == Some(id)
        {
            self.selected = None;
        }
        Ok(())
    }
    fn context(
        &mut self,
        id: ObjectId,
        placement: &transform::Placement,
    ) -> Result<properties::Context> {
        ensure!(
            self.character_properties.is_none(),
            "native character properties require the current world owner"
        );
        Ok(properties::Context {
            actor: id,
            sound_visible: false,
            selected: self.selected == Some(id),
            debug_sound: false,
            position_x: placement.base[0],
            appearance_mode: 0,
            snow: false,
            lot_kind: 1,
        })
    }
    fn viewport(&mut self) -> Result<[i32; 4]> {
        Ok(self.viewport)
    }
    fn visibility(&mut self, id: ObjectId, _: [i32; 4]) -> Result<bool> {
        ensure!(
            self.character_visibility.is_none(),
            "native visibility requires the live model owner"
        );
        Ok(self.visible.as_ref().is_none_or(|v| v.contains(&id)))
    }
    fn visibility_for_model(
        &mut self,
        id: ObjectId,
        resident: &sims_sim::character::Resident,
        model: &sims_content::character_model::State,
        viewport: [i32; 4],
    ) -> Result<bool> {
        match self.character_visibility {
            Some(source) => source.visible(resident, model, viewport),
            None => self.visibility(id, viewport),
        }
    }
    fn visibility_for_initialization(
        &mut self, id: ObjectId, initialized: &sims_sim::pose::initialize::Initialization,
        viewport: [i32; 4],
    ) -> Result<bool> {
        match self.character_visibility {
            Some(source) => source.visible_initialized(initialized, viewport),
            None => self.visibility(id, viewport),
        }
    }
    fn appearance(&mut self, _: ObjectId, request: &character_appearance::Request) -> Result<()> {
        use sims_sim::pose::appearance_selection::Request;
        match request {
            character_appearance::Request::Selection(Request::TextureNotification { .. }) => Ok(()),
            character_appearance::Request::Selection(Request::Tint(tint)) if *tint == [0; 3] => {
                Ok(())
            }
            other => anyhow::bail!("unsupported live presentation service {other:?}"),
        }
    }
    fn saved_texture(&mut self, _: ObjectId) -> Result<()> {
        Ok(())
    }
    fn invalidate(&mut self, _: ObjectId) -> Result<()> {
        Ok(())
    }
    fn property(&mut self, _: ObjectId, request: &properties::Request) -> Result<Response> {
        match request {
            properties::Request::Sound { .. } | properties::Request::TraceSound { .. } => {
                Ok(Response::Done)
            }
            other => anyhow::bail!("unsupported live presentation property {other:?}"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Saved {
    pub version: u32,
    pub source: String,
    pub source_hashes: BTreeMap<String, String>,
    pub options: Options,
    pub modules: BTreeSet<String>,
    /// Exact immutable OBJD linking provenance. Historical saves omit this set
    /// and retain raw definition words when validating their program hash.
    #[serde(
        default,
        skip_serializing_if = "BTreeSet::is_empty",
        deserialize_with = "definition_profile"
    )]
    pub normalized_object_definitions: BTreeSet<String>,
    pub animations: BTreeSet<String>,
    #[serde(
        default,
        skip_serializing_if = "BTreeMap::is_empty",
        deserialize_with = "native_animation_absences"
    )]
    pub native_animation_absences: BTreeMap<String, String>,
    /// Resource dependency reconstructed before validating the program hash.
    /// Older snapshots omitted it and retain their original catalog on restore.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub headline_catalog: bool,
    pub simulation: Snapshot,
    pub assets: character_loading::live::Snapshot,
    pub view: View,
    /// Version2 player saves own imported metadata and mutable resources.
    /// Version1 remains the original-file research checkpoint format.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub managed: Option<Box<managed::Content>>,
}

// BTreeSet's default decoder silently coalesces duplicate owners. Retained
// provenance must be unambiguous rather than normalizing malformed save data.
fn definition_profile<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<BTreeSet<String>, D::Error> {
    let rows = Vec::<String>::deserialize(deserializer)?;
    let count = rows.len();
    let owners: BTreeSet<_> = rows.into_iter().collect();
    if owners.len() != count {
        return Err(serde::de::Error::custom(
            "duplicate normalized definition module",
        ));
    }
    Ok(owners)
}

fn native_animation_absences<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<BTreeMap<String, String>, D::Error> {
    struct Proofs;
    impl<'de> serde::de::Visitor<'de> for Proofs {
        type Value = BTreeMap<String, String>;

        fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.write_str("unique normalized animation names and inventory SHA-256 proofs")
        }

        fn visit_map<M: serde::de::MapAccess<'de>>(
            self,
            mut entries: M,
        ) -> std::result::Result<Self::Value, M::Error> {
            let mut proofs = BTreeMap::new();
            while let Some((name, proof)) = entries.next_entry::<String, String>()? {
                if proofs.insert(name, proof).is_some() {
                    return Err(serde::de::Error::custom(
                        "duplicate native animation absence proof",
                    ));
                }
            }
            validate_native_animation_absences(&proofs).map_err(serde::de::Error::custom)?;
            Ok(proofs)
        }
    }
    deserializer.deserialize_map(Proofs)
}

fn validate_native_animation_absences(proofs: &BTreeMap<String, String>) -> Result<()> {
    for (name, proof) in proofs {
        ensure!(
            !name.is_empty() && *name == name.to_ascii_lowercase(),
            "native animation absence name is not normalized: {name}"
        );
        ensure!(
            proof.len() == 64
                && proof
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
            "native animation absence inventory SHA-256 malformed: {name}"
        );
    }
    Ok(())
}

fn restore_native_animation_absences(
    index: &Index,
    program: &mut Program,
    proofs: &BTreeMap<String, String>,
) -> Result<()> {
    validate_native_animation_absences(proofs)?;
    ensure!(
        program.native_animation_absences.is_empty()
            || program.native_animation_absences == *proofs,
        "restored native animation absence proofs conflict"
    );
    ensure!(
        proofs
            .keys()
            .all(|name| !program.animations.contains_key(name)),
        "saved native animation absence conflicts with a loaded clip"
    );
    if proofs.is_empty() {
        return Ok(());
    }
    let names = proofs.keys().cloned().collect();
    let loaded = sims_content::assets::load_named_native_animations(index, program, &names)?;
    ensure!(
        loaded.catalog_errors.is_empty(),
        "native animation absence inventory scan failed: {:?}",
        loaded.catalog_errors
    );
    ensure!(
        proofs
            .keys()
            .all(|name| !program.animations.contains_key(name)),
        "saved native animation absence now resolves to a clip"
    );
    ensure!(
        loaded.confirmed_absent == *proofs,
        "native animation absence inventory changed or absence unconfirmed"
    );
    program.native_animation_absences = proofs.clone();
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Progress {
    Tick,
    /// A completed native partial pass; no game time or calendar advance.
    PartialPass,
    Dialog,
    /// Native constructor has not returned. The portrait/resource owner must
    /// finish it before calling the ordinary household step again.
    DialogConstruction(sims_vm::ThreadId),
    /// Original0x22 is suspended at a synchronous UI service call.
    UiEffect(sims_vm::ThreadId),
    /// A selection callback entered the installed application dispatcher.
    /// Continue through advance_house_notification before resuming this tick.
    HouseNotification,
}

pub struct Session {
    index: Arc<Index>,
    managed_policy: managed::Policy,
    pub house: House,
    pub program: Program,
    pub simulation: Simulation,
    pub host: character_loading::live::Host<View>,
    pub options: Options,
}
impl Session {
    /// Branch an already-loaded trusted session for deterministic exploration.
    /// This is not a save loader: external bytes still require `restore` and
    /// its source/ownership validation. No opcode, callback or RNG is advanced.
    pub fn fork_for_exploration(&self) -> Result<Self> {
        Ok(Self {
            index: self.index.clone(),
            managed_policy: self.managed_policy,
            house: self.house.clone(),
            program: self.program.clone(),
            simulation: self.simulation.clone(),
            host: self.host.fork_for_exploration()?,
            options: self.options.clone(),
        })
    }

    /// Begin original 4464db after the presenter has handled house-open F6.
    /// Continue with advance_selection; queues are not flushed by ordinary ticks.
    pub fn begin_indicator_refresh(&mut self) -> Result<()> {
        ensure!(
            !self.camera_command_pending()
                && !self.ui_wait_pending()
                && !self.interface_update_pending()
                && !self.control_panel_pending()
                && !self.ui_portrait_pending(),
            "camera command owns presentation work"
        );
        ensure!(
            !self.load_completion_pending(),
            "load completion owns the household"
        );
        ensure!(
            !self.host.presentation.awaiting_house_open,
            "house opening has not completed"
        );
        ensure!(
            !self.view_refresh_pending() && !self.house_notification_pending(),
            "view refresh owns the indicator drain"
        );
        self.simulation
            .begin_native_indicator_refresh(true)
            .map_err(anyhow::Error::msg)
    }
    /// Begin the actual registry selection chain. The caller continues with
    /// `advance_selection`; ordinary ticks cannot overtake its callbacks.
    pub fn begin_selection(&mut self, target: Option<ObjectId>) -> Result<()> {
        ensure!(
            !self.player_input_pending(),
            "player input owns the selected person"
        );
        ensure!(
            !self.camera_command_pending()
                && !self.ui_wait_pending()
                && !self.interface_update_pending()
                && !self.control_panel_pending()
                && !self.ui_portrait_pending(),
            "camera command owns presentation work"
        );
        ensure!(
            !self.load_completion_pending(),
            "load completion owns the household"
        );
        ensure!(
            !self.host.presentation.awaiting_house_open,
            "original house opening must finish before selection"
        );
        ensure!(
            !self.view_refresh_pending() && !self.house_notification_pending(),
            "view refresh must finish before selection"
        );
        self.simulation
            .begin_native_selection(target, true)
            .map_err(anyhow::Error::msg)
    }
    /// Native selection and one asset service, with explicit presentation
    /// camera input. No simulation tick is advanced by this method.
    /// When `house_notification_pending()` becomes true, drive that dispatcher
    /// to completion before calling this method again.
    pub fn advance_selection(
        &mut self,
        camera: sims_sim::pose::portrait::view::Input,
        budget: usize,
    ) -> Result<bool> {
        ensure!(
            !self.player_input_pending(),
            "player input owns the direct factory"
        );
        self.advance_selection_work(camera, budget)
    }
    /// Shared work after the caller has proved its selection ownership. The
    /// public entry above retains its independent player-input exclusion.
    fn advance_selection_work(
        &mut self,
        camera: sims_sim::pose::portrait::view::Input,
        budget: usize,
    ) -> Result<bool> {
        ensure!(
            !self.camera_command_pending()
                && !self.ui_wait_pending()
                && !self.interface_update_pending()
                && !self.control_panel_pending()
                && !self.ui_portrait_pending(),
            "camera command owns presentation work"
        );
        ensure!(
            !self.load_completion_active(),
            "load completion owns the indicator service"
        );
        ensure!(
            !self.postload_pending(),
            "post-load owns the indicator service"
        );
        ensure!(
            !self.view_refresh_pending() && !self.house_notification_pending(),
            "view refresh owns pending selection services"
        );
        ensure!(budget > 0, "selection requires a positive work budget");
        camera.build().map_err(anyhow::Error::msg)?;
        let ordinary = self.host.ordinary.graphics.keys().copied().collect();
        self.simulation
            .advance_native_selection(&self.program, &ordinary, budget)
            .map_err(anyhow::Error::msg)?;
        self.service_selection_assets(camera, budget)
    }
    /// Service the owned selection/indicator call, including calls nested in
    /// deletion. Advancing the enclosing simulation remains the caller's job.
    pub fn service_selection_assets(
        &mut self,
        camera: sims_sim::pose::portrait::view::Input,
        budget: usize,
    ) -> Result<bool> {
        ensure!(
            !self.camera_command_pending()
                && !self.ui_wait_pending()
                && !self.interface_update_pending()
                && !self.control_panel_pending()
                && !self.ui_portrait_pending(),
            "camera command owns presentation work"
        );
        ensure!(
            !self.load_completion_active(),
            "load completion owns the indicator service"
        );
        use sims_content::graphic_update::{Command, Step};
        use sims_sim::{
            lot_selection::Request,
            pose::{graphic_frame, model::Matrix},
        };
        use sims_vm::selection_indicators::GraphicCall;
        ensure!(
            !self.postload_pending(),
            "post-load owns the indicator service"
        );
        ensure!(
            !self.view_refresh_pending() && !self.house_notification_pending(),
            "view refresh owns pending selection services"
        );
        ensure!(budget > 0, "selection requires a positive work budget");
        camera.build().map_err(anyhow::Error::msg)?;
        self.host.presentation.camera = Some(camera);
        self.host.presentation.selected = self
            .simulation
            .native_selected_person()
            .map_err(anyhow::Error::msg)?;
        self.host.update_contexts(&mut self.simulation)?;
        let Some((request, started)) = self.simulation.pending_native_selection() else {
            if self.simulation.pending_removal_graphic().is_some() {
                self.service_removal_graphic(camera)?;
            }
            if self.simulation.selection_work_pending() {
                self.host.service(&mut self.simulation, &mut self.program)?;
            }
            return Ok(!self.simulation.selection_work_pending());
        };
        ensure!(
            self.host.ordinary.pending_updates().all(
                |id| matches!(request, Request::Graphic { object, .. } if started && object == id)
            ),
            "selection overlaps an unrelated ordinary graphic update"
        );
        if matches!(request, Request::Notify(_)) {
            self.begin_selection_broadcast()?;
            return Ok(false);
        }
        if let Request::IndicatorResource(slot) = request {
            let guid = *sims_vm::load_indicators::GUIDS
                .get(slot)
                .context("indicator resource slot invalid")?;
            let present = self.program.modules.values().any(|m| {
                m.object_definition_words.values().any(|w| {
                    w.len() > 20
                        && (u32::from(w[14] as u16) | (u32::from(w[15] as u16) << 16)) == guid
                })
            });
            if !present {
                sims_content::assets::load_object_guid_if_present(
                    &self.index,
                    &mut self.program,
                    guid,
                )?;
            }
            self.simulation
                .architecture
                .as_mut()
                .context("indicator architecture absent")?
                .bind_footprints(&self.program)
                .map_err(anyhow::Error::msg)?;
            let mut definitions = self.program.modules.values().flat_map(|m| {
                m.object_definition_words
                    .iter()
                    .filter_map(move |(&object, w)| {
                        (w.len() > 20
                            && (u32::from(w[14] as u16) | (u32::from(w[15] as u16) << 16)) == guid)
                            .then(|| sims_vm::Definition {
                                source: m.source.clone(),
                                object,
                                guid,
                                slots: w[20] as u16,
                            })
                    })
            });
            let definition = definitions.next();
            ensure!(definitions.next().is_none(), "ambiguous indicator resource");
            self.simulation
                .complete_indicator_resource(&self.program, slot, definition)
                .map_err(anyhow::Error::msg)?;
            return Ok(false);
        }
        ensure!(
            !matches!(request, Request::CreationNotification(_)),
            "indicator creation requires its notification dispatcher"
        );
        if request == Request::Refill {
            let guid = sims_vm::selection_indicators::STAR_GUID;
            let loaded = self.program.modules.values().any(|m| {
                m.object_definition_words.values().any(|w| {
                    w.len() > 20
                        && (u32::from(w[14] as u16) | (u32::from(w[15] as u16) << 16)) == guid
                })
            });
            if !loaded {
                sims_content::assets::load_object_guids(&self.index, &mut self.program, &[guid])?;
                self.simulation
                    .architecture
                    .as_mut()
                    .context("star lot architecture absent")?
                    .bind_footprints(&self.program)
                    .map_err(anyhow::Error::msg)?;
            }
            let mut definitions = self.program.modules.values().flat_map(|m| {
                m.object_definition_words
                    .iter()
                    .filter_map(move |(&object, w)| {
                        (w.len() > 20
                            && (u32::from(w[14] as u16) | (u32::from(w[15] as u16) << 16)) == guid)
                            .then(|| sims_vm::Definition {
                                source: m.source.clone(),
                                object,
                                guid,
                                slots: w[20] as u16,
                            })
                    })
            });
            let definition = definitions
                .next()
                .context("star resource absent after loading")?;
            ensure!(definitions.next().is_none(), "ambiguous star resource");
            self.simulation
                .complete_selection_resource(&self.program, Some(definition))
                .map_err(anyhow::Error::msg)?;
            return Ok(false);
        }
        if let Request::FactoryGraphic(id) = request {
            if !started {
                if let Some(presenter) = &mut self.host.presentation.presenter {
                    // This is a newly constructed object:43189d writes +188=0.
                    // Publish it once, before graphics/Init/E2; never fill in
                    // unknown bytes for arbitrary imported objects here.
                    presenter.inputs.keep_dynamic.insert(id, 0);
                }
            }
            self.host.service_selection_factory_graphic(
                &mut self.simulation,
                &self.program,
                id,
                camera.viewport.map(|n| n as i32),
            )?;
            return Ok(false);
        }
        let Request::Graphic {
            object,
            call,
            placement,
        } = request
        else {
            unreachable!();
        };
        if self.simulation.characters.contains_key(&object) {
            ensure!(
                call == GraphicCall::Refresh && !placement,
                "unsupported person selection graphic call {call:?}"
            );
            if !started {
                self.simulation
                    .start_selection_graphic(request)
                    .map_err(anyhow::Error::msg)?;
            }
            if self
                .simulation
                .refresh_selection_person(&self.program, object, budget)
                .map_err(anyhow::Error::msg)?
            {
                self.simulation
                    .acknowledge_native_selection(request)
                    .map_err(anyhow::Error::msg)?;
            } else {
                self.host
                    .service_view_person(&mut self.simulation, object)?;
            }
            return Ok(false);
        }
        if !started {
            let command = match call {
                GraphicCall::BindTransform(parent) => {
                    let parent =
                        i16::try_from(parent.0).context("invalid parent graphic handle")?;
                    let matrix: Matrix =
                        if let Some(person) = self.simulation.characters.get(&parent) {
                            person.character.model.placement
                        } else {
                            self.host
                                .ordinary
                                .graphics
                                .get(&parent)
                                .context("parent ordinary graphic absent")?
                                .ready
                                .frame
                                .matrix
                        };
                    Command::BindTransform(matrix)
                }
                GraphicCall::TransformControl => Command::TransformControl(true),
                GraphicCall::ReloadModel => Command::Reload { force: true },
                GraphicCall::Refresh => Command::Frame {
                    entry: graphic_frame::Entry::Refresh,
                    only_visible: placement,
                },
                GraphicCall::Update => Command::Frame {
                    entry: graphic_frame::Entry::Ordinary,
                    only_visible: false,
                },
            };
            self.host
                .ordinary
                .graphics
                .get_mut(&object)
                .context("selection ordinary graphic absent")?
                .begin_update(command)?;
            self.simulation
                .start_selection_graphic(request)
                .map_err(anyhow::Error::msg)?;
        }
        let outside = !self
            .simulation
            .native_lot
            .as_ref()
            .context("selection lot absent")?
            .placement
            .links
            .capture_raw(&self.simulation.vm.world, object)
            .map_err(anyhow::Error::msg)?
            .on_lot;
        let step = self.host.service_graphic(
            &mut self.simulation,
            &self.program,
            object,
            character_loading::live::graphic::View {
                viewport: camera.viewport.map(|n| n as i32),
                outside,
                visibility_enabled: true,
                camera: camera.build().map_err(anyhow::Error::msg)?.screen,
                materials: sims_content::graphic_materials::Backend {
                    mode: 0,
                    flags: 0,
                    references: 1,
                    ramp_size: 32,
                },
                normal_material_mode: 0,
            },
        )?;
        match step {
            Step::Complete => self
                .simulation
                .acknowledge_native_selection(request)
                .map_err(anyhow::Error::msg)?,
            Step::Progress => (),
            Step::Pending(p) => {
                anyhow::bail!("native selection graphic callback requires its owner: {p:?}")
            }
        }
        Ok(false)
    }
    /// RemoveFromLot's actual ordinary Animator refresh, before registry unlink.
    pub fn service_removal_graphic(
        &mut self,
        camera: sims_sim::pose::portrait::view::Input,
    ) -> Result<()> {
        ensure!(
            !self.camera_command_pending()
                && !self.ui_wait_pending()
                && !self.interface_update_pending()
                && !self.control_panel_pending()
                && !self.ui_portrait_pending(),
            "camera command owns presentation work"
        );
        ensure!(
            !self.house_notification_pending(),
            "house notification owns graphic services"
        );
        use sims_content::graphic_update::{Command, Step};
        let Some((id, started)) = self.simulation.pending_removal_graphic() else {
            return Ok(());
        };
        if self.host.ordinary.retained.contains(id) {
            return self.host.service_retained_removal_graphic(
                &mut self.simulation,
                &self.program,
                id,
            );
        }
        let matrix = camera.build().map_err(anyhow::Error::msg)?.screen;
        self.host.presentation.camera = Some(camera);
        if !started {
            self.host
                .ordinary
                .graphics
                .get_mut(&id)
                .context("removal ordinary graphic absent")?
                .begin_update(Command::Frame {
                    entry: sims_sim::pose::graphic_frame::Entry::Refresh,
                    only_visible: true,
                })?;
            self.simulation
                .start_removal_graphic(id)
                .map_err(anyhow::Error::msg)?;
        }
        let outside = !self
            .simulation
            .native_lot
            .as_ref()
            .context("removal lot absent")?
            .placement
            .links
            .capture_raw(&self.simulation.vm.world, id)
            .map_err(anyhow::Error::msg)?
            .on_lot;
        match self.host.service_graphic(
            &mut self.simulation,
            &self.program,
            id,
            character_loading::live::graphic::View {
                viewport: camera.viewport.map(|n| n as i32),
                outside,
                visibility_enabled: true,
                camera: matrix,
                materials: sims_content::graphic_materials::Backend {
                    mode: 0,
                    flags: 0,
                    references: 1,
                    ramp_size: 32,
                },
                normal_material_mode: 0,
            },
        )? {
            Step::Complete => self
                .simulation
                .complete_removal_graphic(id)
                .map_err(anyhow::Error::msg)?,
            Step::Progress => (),
            Step::Pending(request) => {
                anyhow::bail!("ordinary removal callback requires its owner: {request:?}")
            }
        }
        Ok(())
    }
    /// 433bb3 forwards to the ordinary Animator's +24(0), invalidating its
    /// placement matrix. This is separate from the +1c physical refresh.
    pub fn service_cursor_appearance(&mut self) -> Result<()> {
        let Some(id) = self
            .simulation
            .pending_cursor_appearance()
            .map_err(anyhow::Error::msg)?
        else {
            return Ok(());
        };
        let graphic = self
            .host
            .ordinary
            .graphics
            .get_mut(&id)
            .context("cursor ordinary graphic absent")?;
        ensure!(
            !graphic.update_pending(),
            "cursor appearance overlaps graphic update"
        );
        graphic.ready.frame.placement.invalidate();
        self.simulation
            .complete_cursor_appearance(id)
            .map_err(anyhow::Error::msg)
    }
    ///43562f calls the ordinary Animator's+20(1),584e03. Its argument is
    ///unused; sentinel position caches and the shared+d9 dirty byte change.
    pub fn service_cursor_release_animator(&mut self) -> Result<()> {
        let Some(h) = self.simulation.pending_cursor_release_animator() else {
            return Ok(());
        };
        let id = self
            .simulation
            .vm
            .world
            .scheduler_storage
            .as_ref()
            .and_then(|s| s.object_for_handle(h))
            .context("release animator lifetime ended")?;
        let graphic = self
            .host
            .ordinary
            .graphics
            .get_mut(&id)
            .context("release ordinary graphic absent")?;
        ensure!(!graphic.update_pending(), "release overlaps graphic update");
        graphic.ready.frame.placement.invalidate();
        graphic.ready.appearance.dirty = true;
        self.simulation
            .complete_cursor_release_animator(id)
            .map_err(anyhow::Error::msg)
    }
    pub fn index(&self) -> &Index {
        &self.index
    }
    pub fn load(index: &Index, source: &str, options: Options) -> Result<Self> {
        Self::load_with_headlines(index, source, options, true)
    }
    fn load_with_headlines(
        index: &Index,
        source: &str,
        options: Options,
        headlines: bool,
    ) -> Result<Self> {
        Self::load_with_resources(index, source, options, headlines, Default::default())
    }
    /// Supply retained skeleton catalogs on repeat loads. Options::crt_seed is
    /// the same process stream at this boundary. This does not yet supply the
    /// incoming object registry or lot-size history for the resize broadcast.
    pub fn load_with_resources(
        index: &Index,
        source: &str,
        options: Options,
        headlines: bool,
        skeletons: sims_content::skeleton_resources::Cache,
    ) -> Result<Self> {
        Self::load_with_startup(index, source, options, headlines, skeletons, None)
    }

    /// The optional incoming owner is already at415e20 entry. It is consumed
    /// through native pre-read work, then the saved objects enter that same VM.
    /// Its CRT state also owns character loading; options.crt_seed initializes
    /// only fresh owners and cannot overwrite a retained process generator.
    pub fn load_with_startup(
        index: &Index,
        source: &str,
        options: Options,
        headlines: bool,
        skeletons: sims_content::skeleton_resources::Cache,
        incoming: Option<sims_content::house_startup::Incoming>,
    ) -> Result<Self> {
        Self::load_with_startup_and_process(
            index, source, options, headlines, skeletons, incoming, None, None,
        )
    }
    pub fn load_with_process_resources(
        index: &Index,
        source: &str,
        options: Options,
        input: sims_content::autonomy_resources::FreshInput,
    ) -> Result<Self> {
        Self::load_with_startup_and_process(
            index,
            source,
            options,
            true,
            Default::default(),
            None,
            Some(input),
            None,
        )
    }
    /// An already retained process and provider are supplied independently.
    /// This never installs fresh process words/configuration into incoming.
    pub fn load_with_retained_process_resources(
        index: &Index,
        source: &str,
        options: Options,
        headlines: bool,
        skeletons: sims_content::skeleton_resources::Cache,
        incoming: sims_content::house_startup::Incoming,
        provider: sims_content::autonomy_resources::Provider,
    ) -> Result<Self> {
        Self::load_with_startup_and_process(
            index,
            source,
            options,
            headlines,
            skeletons,
            Some(incoming),
            None,
            Some(provider),
        )
    }
    fn load_with_startup_and_process(
        index: &Index,
        source: &str,
        options: Options,
        headlines: bool,
        skeletons: sims_content::skeleton_resources::Cache,
        incoming: Option<sims_content::house_startup::Incoming>,
        process: Option<sims_content::autonomy_resources::FreshInput>,
        retained_resources: Option<sims_content::autonomy_resources::Provider>,
    ) -> Result<Self> {
        if retained_resources.is_some() {
            let incoming = incoming
                .as_ref()
                .context("retained provider requires incoming world")?;
            ensure!(
                incoming.vm.world.autonomy_constants.is_some(),
                "retained process owner is unknown"
            );
        }
        let process = process_input_for_load(
            index,
            options.initial_process_profile,
            incoming.is_some(),
            process,
        )?;
        skeletons.validate(index)?;
        ensure!(
            !options.native_load_completion || options.native_postload,
            "load completion requires owned post-load"
        );
        ensure!(
            !(options.native_postload && options.defer_indicators),
            "conflicting post-load profiles"
        );
        let crt_seed = incoming
            .as_ref()
            .map(|owner| {
                owner
                    .vm
                    .crt_rng_state
                    .context("retained startup CRT owner absent")
            })
            .transpose()?
            .unwrap_or(options.crt_seed);
        let house = House::load(index, source)?;
        let mut reader = Reader::new(
            &index,
            &house,
            Settings {
                fade_enabled: true,
                fade_duration: 200,
                sampling_config: 1,
            },
            [0.; 3],
            crt_seed,
            View::default(),
        )?;
        reader.skeletons = skeletons;
        if let Some(input) = process {
            reader.install_process_input(input)?;
        }
        if let Some(provider) = retained_resources {
            reader.install_retained_process_resources(provider)?;
        }
        let context = options
            .application_context
            .unwrap_or(sims_vm::loading::HouseContext {
                simulator: sims_vm::loading::SimulatorContext {
                    game_mode: 0,
                    retained_global25: 0,
                    global30: 1,
                },
                installed_expansions: 0xff,
                performance_class: 2,
            });
        let mut loaded = if let Some(incoming) = incoming {
            house.load_frozen_with_retained_startup(index, context, &mut reader, incoming)?
        } else {
            house.load_frozen_with_native_characters(index, context, &mut reader)?
        };
        let completion = if options.native_load_completion {
            Some(completion::State::prepare(index, &house, &loaded)?)
        } else {
            None
        };
        if !options.defer_indicators && !options.native_postload {
            loaded.initialize_indicators_with_graphics(&index, &mut |vm, program, id| {
                reader.initialize_retained_graphic(vm, program, id)
            })?;
        } else {
            // Loading resident callbacks can extend the program catalog even
            // when indicator resources/factories have not run yet.
            loaded
                .architecture
                .bind_footprints(&loaded.program)
                .map_err(anyhow::Error::msg)?;
        }
        let clock = loaded.execution.lot_clock(&house)?;
        let mut simulation = Simulation::from_loaded_lot(
            loaded.execution.vm,
            loaded.architecture,
            lot_runtime::State {
                placement: loaded.placement,
                membership: loaded.membership,
                indicators: loaded.indicators,
                observers: loaded.observers,
                diagnostics: sims_sim::lot_acceptance::person::Diagnostics {
                    error: 0,
                    blocker: None,
                },
                route: None,
                shoo: None,
                movement: None,
                snap: None,
                find_location: None,
                removal: None,
                snap_offsets: Default::default(),
                completion: None,
                route_paths: Default::default(),
                selected_person: None,
                room_effects: loaded.room_effects,
                routing_policy: options.routing,
            },
        )
        .map_err(anyhow::Error::msg)?;
        simulation.removals = loaded.execution.removals;
        let mut program = loaded.program;
        if headlines {
            sims_content::vm::load_headline_catalog(index, &mut program)?;
        }
        simulation
            .enable_household(&program, clock)
            .map_err(anyhow::Error::msg)?;
        simulation
            .enable_native_residential_autonomy(false)
            .map_err(anyhow::Error::msg)?;
        let mut host = reader.into_live(
            &mut simulation,
            sims_sim::character_update::Settings {
                terrain_scale: None,
                movement: sims_sim::pose::movement::Settings {
                    short_distance: 3.1,
                    turn_rate: 1.,
                    small_turn: 0.5,
                    medium_turn: 1.5,
                    large_turn: 3.,
                    minimum_weight: 0.5,
                    snap_distance: 0.5,
                    rate: sims_sim::pose::ordinary::RateSettings {
                        base: 1.2,
                        positive: 0.2,
                        negative: 0.4,
                        change: 0.2,
                    },
                },
                root_motion: 0,
                person: sims_sim::person_update::Context {
                    only_visible: true,
                    display_animation_names: false,
                    debug_recording: false,
                },
                floor_vector: [0.; 3],
            },
        )?;
        host.ordinary
            .validate_with_retired(&simulation.vm, &simulation.retired_graphics)?;
        host.presentation.awaiting_house_open = options.defer_indicators || options.native_postload;
        host.presentation.loading = completion;
        host.notifications = Some(sims_content::notifications::Software::system_font(
            sims_sim::person_notification::currency::Locale {
                language: 1,
                real_currency: false,
            },
        )?);
        simulation
            .enable_original_environment(
                &program,
                sims_sim::lot_environment::manager::Settings {
                    daylight_multiplier: 1.,
                    light_range: 12.,
                    rooms_enabled: true,
                    // Headless renderer owns six logical model records. No GPU model
                    // allocation is required for their retained references/positions.
                    models: std::array::from_fn(|i| Some(i as u8)),
                },
            )
            .map_err(anyhow::Error::msg)?;
        {
            simulation
                .enable_registry_scheduler(i32::from(house.simulator.scheduler_cursor))
                .map_err(anyhow::Error::msg)?;
        }
        Ok(Self {
            index: Arc::new(index.clone()),
            managed_policy: managed::Policy::HistoricalSchema1,
            house,
            program,
            simulation,
            host,
            options,
        })
    }

    /// Hydrate an original application resource without advancing or editing
    /// simulation state. Old saves retain their fingerprint until the caller
    /// actually resumes; subsequent saves explicitly own this dependency.
    pub fn prepare_headline_catalog(&mut self) -> Result<()> {
        if self
            .program
            .modules
            .get(&self.program.globals)
            .and_then(|m| m.headline_catalog.as_ref())
            .is_none()
        {
            sims_content::vm::load_headline_catalog(&self.index, &mut self.program)?;
        }
        Ok(())
    }

    pub fn ui_portrait_pending(&self) -> bool {
        self.host.presentation.ui_portrait_tick.is_some()
    }
    /// The image backend keeps the resumable getter and native image owners.
    pub fn begin_ui_portrait(&mut self) -> Result<()> {
        ensure!(
            !self.player_input_blocks_ticks(),
            "player input owns the live UI"
        );
        ensure!(
            !self.ui_portrait_pending()
                && !self.host.presentation.awaiting_house_open
                && !self.interface_update_pending()
                && !self.control_panel_pending()
                && !self.camera_command_pending()
                && !self.postload_pending()
                && !self.load_completion_pending()
                && !self.view_refresh_pending()
                && !self.house_notification_pending()
                && !self.simulation.selection_work_pending()
                && !self.host.ordinary.updates_pending()
                && !self.simulation.vm.constructing_dialog()
                && self.simulation.ui_wait_callback.is_none(),
            "UI portrait overlaps synchronous work"
        );
        self.host.presentation.ui_portrait_tick = Some(self.simulation.vm.tick);
        Ok(())
    }
    pub fn finish_ui_portrait(&mut self) -> Result<()> {
        ensure!(
            self.host.presentation.ui_portrait_tick == Some(self.simulation.vm.tick),
            "UI portrait lost its tick owner"
        );
        self.host.presentation.ui_portrait_tick = None;
        Ok(())
    }

    ///4a8895's view-frame prologue calls467094 before drawing. Selection and
    ///indicator attachment can dirty rooms after the loader's initial rebuild;
    ///these scores must be published even when the frame is rendered headlessly.
    ///This does not execute a VM slice or advance the lot clock.
    pub fn update_view_environment(&mut self) -> Result<bool> {
        ensure!(
            !self.camera_command_pending()
                && !self.interface_update_pending()
                && !self.control_panel_pending()
                && !self.ui_portrait_pending()
                && !self.ui_wait_pending()
                && !self.clock_pending()
                && !self.postload_pending()
                && !self.load_completion_pending()
                && !self.house_notification_pending()
                && !self.simulation.selection_work_pending()
                && !self.host.ordinary.updates_pending()
                && self.simulation.vm.scheduled_object_visit.is_none(),
            "suspended caller owns the view-frame environment boundary"
        );
        self.simulation
            .update_original_environment(&self.program)
            .map_err(anyhow::Error::msg)
    }

    /// Complete one tick, preserving original host-service ordering. Dialogs
    /// stay pending until the player answers; no automatic acknowledgment.
    pub fn step(&mut self) -> Result<Progress> {
        ensure!(
            self.host.presentation.application_parent.is_none(),
            "retained application parent owns the household entry"
        );
        ensure!(
            !self.clock_pending(),
            "elapsed frame owns the household update"
        );
        self.step_for_clock()
    }
    /// Observe scheduler instructions and retained native Animate wait polls
    /// without adding a VM owner or changing budgets, services, RNG or yields.
    /// Other nested host-driven interpreters are not implied by this trace.
    pub fn step_with_trace(&mut self, trace: &mut Vec<sims_vm::Trace>) -> Result<Progress> {
        ensure!(
            self.host.presentation.application_parent.is_none(),
            "retained application parent owns the household entry"
        );
        ensure!(
            !self.clock_pending(),
            "elapsed frame owns the household update"
        );
        self.step_for_clock_observed(Some(trace))
    }
    pub(super) fn step_for_clock(&mut self) -> Result<Progress> {
        self.step_for_clock_observed(None)
    }
    /// Read-only dependent-service cuts for retained-state/replay verification.
    /// The production and observed paths share budgets and service ordering.
    pub fn step_with_dependent_observer(
        &mut self,
        trace: &mut Vec<sims_vm::Trace>,
        observer: &mut dyn FnMut(&Self) -> Result<()>,
    ) -> Result<Progress> {
        ensure!(
            self.host.presentation.application_parent.is_none(),
            "retained application parent owns the household entry"
        );
        ensure!(
            !self.clock_pending(),
            "elapsed frame owns the household update"
        );
        self.step_for_clock_monitored(Some(trace), Some(observer))
    }
    fn step_for_clock_observed(
        &mut self,
        trace: Option<&mut Vec<sims_vm::Trace>>,
    ) -> Result<Progress> {
        self.step_for_clock_monitored(trace, None)
    }
    fn step_for_clock_monitored(
        &mut self,
        mut trace: Option<&mut Vec<sims_vm::Trace>>,
        mut observer: Option<&mut dyn FnMut(&Self) -> Result<()>>,
    ) -> Result<Progress> {
        ensure!(
            !self.person_draw_preparation_pending(),
            "person draw preparation must return before household ticks"
        );
        if let Some(menu) = &self.host.presentation.live_menu {
            menu.validate(self)?;
        }
        ensure!(
            !self.player_input_blocks_ticks(),
            "player input must return before household ticks"
        );
        ensure!(
            !self.interface_update_pending()
                && !self.control_panel_pending()
                && !self.ui_portrait_pending(),
            "interface update must return before household ticks"
        );
        ensure!(
            !self.camera_command_pending()
                && !self.ui_wait_pending()
                && !self.interface_update_pending()
                && !self.control_panel_pending()
                && !self.ui_portrait_pending(),
            "camera command must return before household ticks"
        );
        ensure!(
            !self.load_completion_pending(),
            "original load completion must return before ticks"
        );
        ensure!(
            !self.host.presentation.awaiting_house_open,
            "original house opening must finish before household ticks"
        );
        ensure!(
            !self.view_refresh_pending() && !self.house_notification_pending(),
            "native view refresh must finish before household ticks resume"
        );
        const BUDGET: usize = 100_000;
        for _ in 0..1000 {
            if let Some((&thread, _)) = self
                .simulation
                .vm
                .threads
                .iter()
                .find(|(_, t)| matches!(t.status, sims_vm::Status::AwaitingUiEffect(_)))
            {
                return Ok(Progress::UiEffect(thread));
            }
            if self.simulation.pending_removal_graphic().is_some() {
                let camera = self.host.presentation.camera.unwrap_or(
                    sims_sim::pose::portrait::view::Input {
                        viewport: self.host.presentation.viewport.map(|n| n as u32),
                        center: [0, 0],
                        zoom: 3,
                        rotation: 0,
                    },
                );
                self.service_removal_graphic(camera)?;
                continue;
            }
            if self.simulation.native_selection.is_none()
                && self.simulation.pending_native_selection().is_some()
            {
                let camera = self.host.presentation.camera.unwrap_or(
                    sims_sim::pose::portrait::view::Input {
                        viewport: self.host.presentation.viewport.map(|n| n as u32),
                        center: [0, 0],
                        zoom: 3,
                        rotation: 0,
                    },
                );
                self.service_selection_assets(camera, BUDGET)?;
                if self.house_notification_pending() {
                    return Ok(Progress::HouseNotification);
                }
                continue;
            }
            ensure!(
                !self.host.ordinary.updates_pending() || self.host.dependent_pending(),
                "ordinary graphic service must finish before the household scheduler resumes"
            );
            if let Some((&thread, _)) = self
                .simulation
                .vm
                .threads
                .iter()
                .find(|(_, t)| matches!(t.status, sims_vm::Status::AwaitingDialog(_)))
            {
                return Ok(Progress::DialogConstruction(thread));
            }
            // A pending modal stays byte-for-byte unchanged. Resource hydration
            // is part of resuming play, never a repair of a faulted thread.
            if self.simulation.vm.world.dialogs.modal.is_none() {
                self.prepare_headline_catalog()?;
            }
            self.host.update_contexts(&mut self.simulation)?;
            let mut host_polls = Vec::new();
            let result = if trace.is_some() {
                self.simulation.run_current_tick_with_host_polls(&self.program, BUDGET, &mut host_polls)
            } else {
                self.simulation.run_current_tick(&self.program, BUDGET)
            };
            if let (Ok(report), Some(trace)) = (&result, trace.as_deref_mut()) {
                let mut cursor = 0;
                for poll in host_polls {
                    trace.extend_from_slice(&report.trace[cursor..poll.before_instruction]);
                    trace.push(poll.instruction);
                    cursor = poll.before_instruction;
                }
                trace.extend_from_slice(&report.trace[cursor..]);
            }
            if let Some((&thread, _)) = self
                .simulation
                .vm
                .threads
                .iter()
                .find(|(_, t)| matches!(t.status, sims_vm::Status::AwaitingUiEffect(_)))
            {
                result.map_err(anyhow::Error::msg)?;
                return Ok(Progress::UiEffect(thread));
            }
            self.host.presentation.selected = self
                .simulation
                .native_selected_person()
                .map_err(anyhow::Error::msg)?;
            if let Some((&thread, _)) = self
                .simulation
                .vm
                .threads
                .iter()
                .find(|(_, t)| matches!(t.status, sims_vm::Status::AwaitingDialog(_)))
            {
                result.map_err(anyhow::Error::msg)?;
                return Ok(Progress::DialogConstruction(thread));
            }
            if let Some((parent, _)) = self
                .simulation
                .pending_character_services()
                .into_iter()
                .find(|(_, r)| *r == sims_sim::pose::refresh::Request::OrdinaryCarriedObject)
            {
                let camera = self
                    .host
                    .presentation
                    .camera
                    .context("dependent graphic camera owner absent")?;
                let child = self.simulation.vm.world.objects[&parent].slots[1]
                    .context("dependent graphic slot absent")?;
                let links = &self
                    .simulation
                    .native_lot
                    .as_ref()
                    .context("dependent lot absent")?
                    .placement
                    .links;
                let outside = !links
                    .capture_raw(&self.simulation.vm.world, child)
                    .map_err(anyhow::Error::msg)?
                    .on_lot;
                self.host.service_dependent(
                    &mut self.simulation,
                    &self.program,
                    parent,
                    character_loading::live::graphic::View {
                        viewport: camera.viewport.map(|v| v as i32),
                        outside,
                        visibility_enabled: self
                            .host
                            .presentation
                            .character_visibility
                            .is_none_or(|source| source.enabled),
                        camera: match self.host.presentation.character_visibility {
                            Some(source) => source.camera,
                            None => camera.build().map_err(anyhow::Error::msg)?.screen,
                        },
                        materials: sims_content::graphic_materials::Backend {
                            mode: 0,
                            flags: 0,
                            references: 1,
                            ramp_size: 32,
                        },
                        normal_material_mode: 0,
                    },
                )?;
                if let Some(f) = observer.as_deref_mut() {
                    f(self)?;
                }
                continue;
            }
            if self.host.service(&mut self.simulation, &mut self.program)? > 0 {
                continue;
            }
            let step = result.map_err(anyhow::Error::msg)?;
            match step.stop {
                StepStop::Dialog => return Ok(Progress::Dialog),
                StepStop::Budget
                    if step.trace.len() >= BUDGET
                        || self.simulation.deletion_entry_pending()
                        || self.simulation.pending_removal_graphic().is_some() =>
                {
                    continue;
                }
                StepStop::Budget => anyhow::bail!(
                    "unsupported pending live services: character={:?}; person={:?}; refresh={:?}",
                    self.simulation.pending_character_updates(),
                    self.simulation.pending_person_services(),
                    self.simulation.pending_character_services()
                ),
                StepStop::Faulted => {
                    let faults: BTreeMap<_, _> = self
                        .simulation
                        .vm
                        .threads
                        .iter()
                        .filter_map(|(id, t)| {
                            if let sims_vm::Status::Faulted(fault) = &t.status {
                                Some((id, fault))
                            } else {
                                None
                            }
                        })
                        .collect();
                    anyhow::bail!("VM fault: {faults:?}");
                }
                _ => {
                    self.host.present_notifications(
                        self.simulation.vm.tick as u32,
                        &self
                            .simulation
                            .characters
                            .keys()
                            .copied()
                            .collect::<Vec<_>>(),
                    )?;
                    let advanced = self
                        .simulation
                        .finish_registry_pass()
                        .map_err(anyhow::Error::msg)?;
                    self.simulation
                        .update_original_environment(&self.program)
                        .map_err(anyhow::Error::msg)?;
                    return Ok(if advanced {
                        Progress::Tick
                    } else {
                        Progress::PartialPass
                    });
                }
            }
        }
        anyhow::bail!("live session service budget exceeded")
    }

    pub fn save(&self) -> Saved {
        Saved {
            version: 1,
            source: self.house.source.clone(),
            source_hashes: self.house.sources.clone(),
            options: self.options.clone(),
            modules: self.program.modules.keys().cloned().collect(),
            normalized_object_definitions: self
                .program
                .modules
                .iter()
                .filter(|(_, module)| module.normalized_object_definitions)
                .map(|(name, _)| name.clone())
                .collect(),
            animations: self.program.animations.keys().cloned().collect(),
            native_animation_absences: self.program.native_animation_absences.clone(),
            headline_catalog: self
                .program
                .modules
                .get(&self.program.globals)
                .is_some_and(|m| m.headline_catalog.is_some()),
            simulation: Snapshot::capture(&self.program, &self.simulation),
            assets: self.host.snapshot(),
            view: self.host.presentation.clone(),
            managed: None,
        }
    }
    /// Reconstruct the exact program catalog from original resources before
    /// validating its fingerprint and restoring both continuation owners.
    pub fn restore(index: &Index, saved: Saved) -> Result<Self> {
        if saved.version == 2 {
            return managed::restore(index, saved);
        }
        ensure!(saved.managed.is_none(), "managed payload requires version2");
        ensure!(
            saved.simulation.state.native_scheduler.is_some()
                && saved.simulation.state.fixture_driver.is_none(),
            "legacy household snapshot has no original registry continuation; reload its original house to start a new session"
        );
        ensure!(
            saved.version == 1,
            "unsupported original-session snapshot version"
        );
        let mut session =
            Self::load_with_headlines(index, &saved.source, saved.options, saved.headline_catalog)?;
        ensure!(
            session.house.sources == saved.source_hashes,
            "original household sources changed"
        );
        for source in &saved.modules {
            if session.program.modules.contains_key(source) {
                continue;
            }
            let loaded = sims_content::vm::load_program(index, source, "Global.iff")?;
            ensure!(
                loaded.globals == session.program.globals,
                "restored global resource differs"
            );
            for (name, module) in loaded.modules {
                if let Some(old) = session.program.modules.get(&name) {
                    ensure!(
                        old.content_hash == module.content_hash,
                        "restored module changed: {name}"
                    );
                } else {
                    session.program.modules.insert(name, module);
                }
            }
        }
        sims_content::vm::restore_definition_profiles(
            index,
            &mut session.program,
            &saved.normalized_object_definitions,
        )?;
        let loaded = sims_content::assets::load_named_native_animations(
            index,
            &mut session.program,
            &saved.animations,
        )?;
        ensure!(
            loaded.unresolved.is_empty(),
            "saved animation resources missing: {:?}",
            loaded.unresolved
        );
        restore_native_animation_absences(
            index,
            &mut session.program,
            &saved.native_animation_absences,
        )?;
        ensure!(
            session
                .program
                .modules
                .keys()
                .cloned()
                .collect::<BTreeSet<_>>()
                == saved.modules
                && session
                    .program
                    .animations
                    .keys()
                    .cloned()
                    .collect::<BTreeSet<_>>()
                    == saved.animations,
            "restored program catalog differs"
        );
        // Version1 predates the retained STR# dependency catalog. Reconstruct
        // its exact earlier whitelist only when the complete saved fingerprint
        // agrees; never rewrite a mismatching snapshot hash to accept it.
        if saved.simulation.program_hash != session.program.fingerprint() {
            let mut previous = session.program.clone();
            let globals = previous.modules.get_mut(&previous.globals).unwrap();
            for id in sims_content::vm::PREVIOUSLY_UNLINKED_ANIMATION_TABLES {
                globals.strings.remove(&id);
            }
            if saved.simulation.program_hash == previous.fingerprint() {
                session.program = previous;
            }
        }
        session.simulation = saved
            .simulation
            .restore(&session.program)
            .map_err(anyhow::Error::msg)?;
        ensure!(
            session.simulation.native_scheduler.is_some()
                && session.simulation.fixture_driver.is_none(),
            "saved household registry owner is absent or replaced by a fixture"
        );
        if let (Some(saved), Some(loaded)) =
            (&saved.view.loading, &session.host.presentation.loading)
        {
            saved.validate_source(loaded)?;
        }
        session.host = character_loading::live::Host::restore(index, saved.view, saved.assets)?;
        session.validate_restored()?;
        sims_content::vm::prepare_native_animation_tables(index, &mut session.program)?;
        Ok(session)
    }
    fn validate_restored(&mut self) -> Result<()> {
        let session = self;
        session.validate_neighborhood_input()?;
        if let Some(draw) = &session.host.presentation.person_draw {
            draw.validate(session)?;
            ensure!(
                session.person_draw_preparation_ready(),
                "restored draw preparation overlaps another caller"
            );
        }
        ensure!(
            session
                .simulation
                .household
                .as_ref()
                .and_then(|h| h.valuation.as_ref())
                .is_some()
                == session.host.presentation.valuation_catalog.is_some(),
            "saved valuation lost its catalog owner"
        );
        if let Some(catalog) = &session.host.presentation.valuation_catalog {
            catalog.validate(&session.index)?;
        }
        let families = session
            .simulation
            .vm
            .world
            .person
            .as_ref()
            .context("saved original neighborhood absent")?;
        ensure!(
            families.family_metadata.keys().eq(families.families.keys()),
            "saved original family metadata absent or incomplete"
        );
        ensure!(
            session
                .simulation
                .native_lot
                .as_ref()
                .is_some_and(|n| n.routing_policy == session.options.routing),
            "saved routing policy differs from session options"
        );
        ensure!(
            session.host.presentation.loading.is_some() == session.options.native_load_completion,
            "saved load-completion owner differs from profile"
        );
        if let Some(loading) = &session.host.presentation.loading {
            loading.validate(&session)?;
        }
        ensure!(
            !session.host.presentation.awaiting_house_open
                || session.options.defer_indicators
                || session.options.native_postload,
            "saved house-opening stage differs from load profile"
        );
        if let Some(postload) = &session.host.presentation.postload {
            ensure!(
                session.options.native_postload,
                "saved post-load owner differs from import profile"
            );
            postload.validate(&session.simulation)?;
        }
        ensure!(
            !session.options.native_postload
                || session.host.presentation.postload.is_some()
                || session.simulation.native_selection.is_none(),
            "saved post-load child lost its session owner"
        );
        ensure!(
            !session.options.native_postload
                || session.host.presentation.awaiting_house_open
                || session
                    .host
                    .presentation
                    .postload
                    .as_ref()
                    .is_some_and(postload::State::complete),
            "saved house bypassed its post-load tail"
        );
        if let Some(presenter) = &session.host.presentation.presenter {
            presenter
                .validate_simulation(&session.simulation)
                .map_err(anyhow::Error::msg)?;
        }
        if let Some(notifications) = &session.host.presentation.notifications {
            notifications.validate(&session.host.presentation, &session.simulation)?;
        }
        if let Some(properties) = &session.host.presentation.property_refresh {
            properties.validate(session)?;
        }
        session.validate_cursor_focus()?;
        if let Some(purchase) = &session.host.presentation.purchase_cursor {
            purchase.validate(&session)?;
            ensure!(
                !purchase.move_owned()
                    || session
                        .host
                        .presentation
                        .move_tool
                        .as_ref()
                        .is_some_and(move_tool::State::preview_creating),
                "saved purchase lost its Move caller"
            );
        }
        if let Some(tool) = &session.host.presentation.move_tool {
            tool.validate(&session)?;
        }
        session.validate_tool_selection_owner()?;
        if let Some(inputs) = &session.host.presentation.retained_tool_inputs {
            inputs.validate(session)?;
        }
        if let Some(frontend) = &session.host.presentation.frontend_context {
            frontend.validate(session)?;
        }
        session.validate_move_button_owner()?;
        session.validate_publication()?;
        if let Some(budget) = &session.host.presentation.object_budget {
            budget.validate(&session)?;
        }
        if let Some(wait) = &session.host.presentation.ui_wait {
            wait.validate(&session)?;
        } else {
            ensure!(
                session.simulation.ui_wait_callback.is_none(),
                "UI wait callback lost its application owner"
            );
        }
        if let Some(window) = &session.host.presentation.family_window {
            window.validate(session)?;
        }
        if let Some(update) = &session.host.presentation.interface_update {
            update.validate(&session)?;
        }
        if let Some(clock) = &session.host.presentation.clock {
            clock.validate(&session)?;
        }
        if let Some(parent) = &session.host.presentation.application_parent {
            parent.validate(
                session.host.presentation.clock.as_ref(),
                session.simulation.vm.tick,
            )?;
        }
        if let Some(transport) = &session.host.presentation.transport {
            transport.validate(&session)?;
        }
        ensure!(
            session
                .host
                .presentation
                .ui_portrait_tick
                .is_none_or(|t| t == session.simulation.vm.tick),
            "saved UI portrait crossed a lot tick"
        );
        ensure!(
            session.simulation.ground_placement.is_none() || session.ground_input_pending(),
            "ground placement lost its session owner"
        );
        ensure!(
            session
                .host
                .presentation
                .picker_target
                .is_none_or(|id| id > 0),
            "invalid saved picker target"
        );
        if let Some(input) = &session.host.presentation.ground_input {
            input.validate(&session)?;
        }
        if let Some(input) = &session.host.presentation.object_input {
            input.validate(&session)?;
        }
        if let Some(menu) = &session.host.presentation.live_menu {
            menu.validate(&session)?;
        }
        if let Some(panel) = &session.host.presentation.control_panel {
            panel.validate(&session)?;
        }
        if let Some(command) = &session.host.presentation.camera_command {
            command.validate(&session)?;
        }
        // Older session saves can retain a completed notification record while
        // selection still uses the explicit Globals-only host profile. Preserve
        // that profile on restore; installing a new broadcast opts into the
        // application dispatcher. An enabled binding must always retain it.
        ensure!(
            !session.simulation.native_selection_broadcasts
                || session.host.presentation.notifications.is_some(),
            "saved selection dispatcher binding differs"
        );
        // Earlier diagnostic saves have no retained ordinary graphics. Keep
        // that absence explicit; do not rebuild them or replay CRT draws here.
        // A service requiring those assets must reject the missing owner.
        let before = session.simulation.clone();
        session.host.update_contexts(&mut session.simulation)?;
        ensure!(
            session.simulation == before,
            "saved presentation context differs from simulation"
        );
        Ok(())
    }
}

/// Fresh-profile selection precedes loading. Retained incoming owners, including
/// unknown historical process state, are never repaired from a new bootstrap.
fn process_input_for_load(
    index: &Index,
    profile: Option<sims_content::autonomy_resources::InitialProfile>,
    retained: bool,
    explicit: Option<sims_content::autonomy_resources::FreshInput>,
) -> Result<Option<sims_content::autonomy_resources::FreshInput>> {
    ensure!(
        !retained || explicit.is_none(),
        "fresh process input overlaps retained incoming world"
    );
    if retained || explicit.is_some() {
        return Ok(explicit);
    }
    profile.map(|profile| profile.prepare(index)).transpose()
}
#[cfg(test)]
#[path = "original_session/native_animation_absence_tests.rs"]
mod native_animation_absence_tests;

#[cfg(test)]
mod process_profile_tests {
    use super::*;
    #[test]
    fn named_default_is_explicit_but_historical_missing_field_stays_unknown() {
        let options = Options::default();
        assert!(options.initial_process_profile.is_some());
        let mut value = serde_json::to_value(&options).unwrap();
        value
            .as_object_mut()
            .unwrap()
            .remove("initial_process_profile");
        let old: Options = serde_json::from_value(value).unwrap();
        assert!(old.initial_process_profile.is_none());
    }
    #[test]
    fn retained_and_historical_none_do_not_read_or_construct_fresh_provider() {
        let index = Index {
            root: "missing-install".into(),
            resources: vec![],
            archives: 0,
            diagnostics: vec![],
        };
        let profile = Options::default().initial_process_profile;
        assert!(
            process_input_for_load(&index, profile, true, None)
                .unwrap()
                .is_none()
        );
        assert!(
            process_input_for_load(&index, None, false, None)
                .unwrap()
                .is_none()
        );
        assert!(
            process_input_for_load(&index, profile, false, None)
                .unwrap_err()
                .to_string()
                .contains("provider missing or ambiguous")
        );
    }
}
