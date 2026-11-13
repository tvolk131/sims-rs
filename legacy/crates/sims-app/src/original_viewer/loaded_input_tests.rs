//! Authored asset-free managed-fixture controls, ALL UNRUN. Included below
//! exploration so authored_fixture's real Application type and imports remain
//! in their existing scope. Tests enter Input::read and Viewer::load_with_index,
//! not a typed fixture masquerading as a successful observed player load.
use super::*;
use clap::Parser;
use super::super::super::support::window_session::loaded_input::Account;
use sims_runtime::original_session::player_input::Input;
use std::io::Write;
#[path = "exploration/service_capture/authored_fixture.rs"]
mod fixture;
fn bounded_control(control: fn() -> Result<()>) -> Result<()> {
    std::thread::Builder::new().stack_size(64 * 1024 * 1024).spawn(control)?
        .join().map_err(|_| anyhow::anyhow!("authored loaded-input control panicked"))?
}
struct InputFile { path: PathBuf, raw: Vec<u8> }
impl InputFile {
    fn new(raw: Vec<u8>) -> Result<Self> {
        let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_nanos();
        let path = std::env::temp_dir().join(format!("authored-player-input-{}-{stamp}.json", std::process::id()));
        let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(&path)?;
        file.write_all(&raw)?;
        Ok(Self { path, raw })
    }
    fn value(value: &Value) -> Result<Self> { Self::new(serde_json::to_vec(value)?) }
}
impl Drop for InputFile { fn drop(&mut self) { let _ = std::fs::remove_file(&self.path); } }
// This local extension leaves service_capture's deliberately renderer-incomplete
// fixture (and its missing-renderer assertions) unchanged. Every added payload
// below is authored here; the filenames are only the renderer's lookup keys.
fn player_resource(index: &mut Index, group: &str, name: &str, bytes: Vec<u8>) {
    let locator = format!("GameData_ALL_Recursive/{group}/{group}.far::{name}");
    assert!(!index.resources.iter().any(|resource| resource.locator.eq_ignore_ascii_case(&locator)));
    index.resources.push(sims_content::Resource {
        source: index.root.join(format!("GameData_ALL_Recursive/{group}/{name}")),
        locator, name: name.into(), offset: 0,
        size: bytes.len() as u64, unpacked_size: bytes.len() as u64,
        archived: false, owned: Some(Arc::from(bytes)),
    });
}
fn player_bcf_string(bytes: &mut Vec<u8>, value: &str) {
    assert!(value.is_ascii() && value.len() < 255);
    bytes.push(value.len() as u8); bytes.extend(value.as_bytes());
}
fn player_skeleton() -> Vec<u8> {
    let mut bytes = Vec::from(1_u32.to_le_bytes()); // one skeleton
    player_bcf_string(&mut bytes, "adult"); bytes.extend(1_u32.to_le_bytes());
    player_bcf_string(&mut bytes, "ROOT"); player_bcf_string(&mut bytes, "NULL");
    bytes.extend(0_u32.to_le_bytes()); // no bone properties
    for value in [0_f32, 0., 0., 0., 0., 0., 1.] { bytes.extend(value.to_le_bytes()); }
    for flag in [1_u32, 1, 0] { bytes.extend(flag.to_le_bytes()); }
    bytes.extend(0_i32.to_le_bytes()); bytes.extend(0_f32.to_le_bytes()); // no wiggle
    bytes.extend(0_u32.to_le_bytes()); bytes.extend(0_u32.to_le_bytes()); // no appearances/animations
    bytes
}
fn player_appearance(name: &str) -> Vec<u8> {
    let mut bytes = Vec::from(0_u32.to_le_bytes()); bytes.extend(1_u32.to_le_bytes());
    player_bcf_string(&mut bytes, name);
    for value in [0_u32, 0, 1] { bytes.extend(value.to_le_bytes()); } // kind, reserved, binding count
    player_bcf_string(&mut bytes, "ROOT"); player_bcf_string(&mut bytes, "authored-player-triangle");
    for value in [0_u32, 0, 0] { bytes.extend(value.to_le_bytes()); } // censor, reserved, animations
    bytes
}
fn player_mesh() -> Vec<u8> {
    let mut bytes = vec![];
    player_bcf_string(&mut bytes, "authored-player-triangle");
    player_bcf_string(&mut bytes, "authored-player-texture");
    bytes.extend(1_u32.to_le_bytes()); player_bcf_string(&mut bytes, "ROOT");
    for value in [1_u32, 0, 1, 2, 1, 0, 0, 3, 0, 0, 3] { bytes.extend(value.to_le_bytes()); }
    // One triangle, one ROOT binding covering all three vertices, no blends.
    for value in [0_f32, 0., 1., 0., 0., 1.] { bytes.extend(value.to_le_bytes()); }
    bytes.extend(0_u32.to_le_bytes()); bytes.extend(3_u32.to_le_bytes());
    for position in [[0_f32, 0., 0.], [0.25, 0., 0.], [0., 0.25, 0.]] {
        for value in position.into_iter().chain([0_f32, 0., 1.]) { bytes.extend(value.to_le_bytes()); }
    }
    bytes
}
fn player_bitmap() -> Result<Vec<u8>> {
    let pixels = image::RgbImage::from_pixel(2, 2, image::Rgb([23, 47, 71]));
    let mut bytes = std::io::Cursor::new(Vec::new());
    pixels.write_to(&mut bytes, image::ImageFormat::Bmp)?;
    Ok(bytes.into_inner())
}
fn player_iff(chunks: Vec<([u8; 4], u16, Vec<u8>)>) -> Vec<u8> {
    let mut bytes = b"IFF FILE 2.5:TYPE FOLLOWED BY SIZE\0 JAMIE DOORNBOS & MAXIS 1".to_vec();
    bytes.resize(64, 0);
    for (tag, identifier, payload) in chunks {
        bytes.extend(tag); bytes.extend(((76 + payload.len()) as u32).to_be_bytes());
        bytes.extend(identifier.to_be_bytes()); bytes.extend(0_u16.to_be_bytes());
        bytes.extend([0; 64]); bytes.extend(payload);
    }
    bytes
}
fn player_pool_sprite() -> Vec<u8> {
    let mut bytes = vec![];
    for value in [1000_u32, 1, 1, 16] { bytes.extend(value.to_le_bytes()); }
    bytes.extend(1_u16.to_le_bytes()); bytes.extend(1_u16.to_le_bytes()); // one opaque pixel
    bytes.extend(7_u32.to_le_bytes()); bytes.extend([0; 8]); // flags, palette/reserved, zero offsets
    bytes.extend(6_u16.to_le_bytes()); bytes.extend(0x2001_u16.to_le_bytes());
    bytes.extend([8, 0]); bytes.extend(0xa000_u16.to_le_bytes()); // depth, palette index, end
    bytes
}
fn player_pool_floors() -> Result<Vec<u8>> {
    let mut palette = Vec::from(0_u32.to_le_bytes()); palette.extend(1_u32.to_le_bytes());
    palette.extend([0; 8]); palette.extend([23, 47, 71]);
    let mut chunks = vec![(*b"PALT", 1, palette)];
    // The base managed simulation genuinely has pool=0 in all 36 cells. Keep
    // it unchanged and supply every surface/rim/corner lookup the real binder
    // requires at all three zooms, including shapes absent in this exact view.
    for zoom in 0..3_u16 {
        for shape in 0..21_u16 {
            let id = match shape {
                0..=15 => 1024 + zoom * 16 + shape,
                16 => 12 + zoom * 256,
                _ => 1072 + zoom * 4 + shape - 17,
            };
            chunks.push((*b"SPR2", id, player_pool_sprite()));
        }
    }
    let bytes = player_iff(chunks);
    let iff = sims_formats::iff::Iff::parse(bytes.clone())?;
    for chunk in iff.chunks.iter().filter(|chunk| chunk.tag == *b"SPR2") {
        let raw = iff.data(chunk); let set = sims_formats::sprite::SpriteSet::parse(raw)?;
        let frame = set.decode(raw, 0, &iff)?;
        ensure!(set.frames.len() == 1 && frame.width == 1 && frame.height == 1
            && frame.x == 0 && frame.y == 0 && frame.rgba == [23, 47, 71, 255],
            "authored pool sprite did not decode through its real palette");
    }
    Ok(bytes)
}
fn player_index() -> Result<Index> {
    let mut index = fixture::index()?;
    let skeleton = player_skeleton();
    let decoded = sims_formats::avatar::read_bcf(&skeleton)?;
    ensure!(decoded.skeletons.len() == 1 && decoded.skeletons[0].bones.len() == 1
        && decoded.skeletons[0].bones[0].name == "ROOT", "authored skeleton codec differs");
    player_resource(&mut index, "Animation", "adult-skeleton.cmx.bcf", skeleton);
    let mesh = player_mesh(); let decoded = sims_formats::avatar::read_bmf(&mesh)?;
    ensure!(decoded.vertices.len() == 3 && decoded.triangles.len() == 1
        && decoded.bindings.len() == 1 && decoded.vertices.iter().all(|vertex| vertex.bone == 0),
        "authored mesh must really bind its complete triangle");
    player_resource(&mut index, "Animation", "authored-player-triangle.bmf", mesh);
    for name in ["b002mafit_01", "nmfit_01", "pajamamfit_01", "c001ma_ross", "hmlo", "hmro"] {
        let bytes = player_appearance(name); let decoded = sims_formats::avatar::read_bcf(&bytes)?;
        ensure!(decoded.appearances.len() == 1 && decoded.appearances[0].name == name
            && decoded.appearances[0].bindings.len() == 1, "authored appearance codec differs");
        player_resource(&mut index, "Animation", &format!("{name}.cmx.bcf"), bytes);
    }
    for name in ["B002MAFitlgt_Ross.bmp", "B002MAFitlgt_casual.bmp", "NMFitlgt_01.bmp",
        "PajamaMlgt_01.bmp", "C001MAlgt_Ross.bmp", "HUAOlgt.bmp"] {
        player_resource(&mut index, "Textures", name, player_bitmap()?);
    }
    player_resource(&mut index, "Floors", "floors.iff", player_pool_floors()?);
    // No wall segments exist in this fixture; a valid empty shape catalog is
    // still a required owned resource, rather than a bypass of wall binding.
    player_resource(&mut index, "Walls", "walls.iff", player_iff(vec![]));
    ensure!(index.resources.iter().all(|resource| resource.owned.is_some()));
    Ok(index)
}
fn player_house_arrays(session: &mut Session) -> Result<()> {
    let lot = session.simulation.architecture.as_ref().and_then(|architecture| architecture.original.as_ref())
        .context("authored player lot absent")?;
    ensure!(lot.size == session.house.width && lot.size == session.house.height
        && lot.pool.len() == lot.size * lot.size && lot.pool.iter().all(|value| *value == 0)
        && lot.floors.iter().all(|floor| floor.source.walls.iter().all(|wall| wall.segments == 0))
        && session.house.floors.iter().all(|floor| floor.walls.iter().all(|wall| wall.segments == 0)),
        "authored player surface resources no longer describe this fixture");
    ensure!(!session.house.arrays.contains_key(&6) && !session.house.arrays.contains_key(&9),
        "authored player surface arrays must not overwrite existing imported data");
    for (id, data) in [(6, vec![0; lot.size * lot.size]), (9, lot.pool.clone())] {
        session.house.arrays.insert(id, sims_formats::lot::Array {
            width: lot.size, height: lot.size, stride: 1, encoding: 0, data,
        });
    }
    Ok(())
}
fn envelope() -> Result<(Index, Value)> {
    let index = player_index()?;
    let mut session = fixture::session(&index)?;
    player_house_arrays(&mut session)?;
    let client = DialogClient::new(&mut session)?;
    // The existing service fixture has no player roster. This owned authored
    // roster supplies the loader's actual resident validation without private
    // character assets, scheduler changes, or invented identity observations.
    session.house.household = Some(serde_json::from_value(json!({
        "id":1,"name":"authored player envelope",
        "family":{"version":7,"house":7,"number":1,"budget":0,"architecture_value":0,
            "friends":0,"flags":0,"members":[1],"tail":[]},
        "residents":[{"guid":1,"neighbor":1,"object":4,"name":"authored",
            "character_source":fixture::GLOBAL,"body_strings":[]}]
    }))?);
    let value = json!({"session":session.save_managed()?,"dialog_client":client.save_managed(&session)?,
        "camera_device":null,"floor":1,"zoom":0.55,"resident":4,"selected":null,
        "tutorial_highlight":null,"viewport_enter_requested":false,"move_inputs":[]});
    Ok((index, value))
}
fn viewer(account: Option<&Account>) -> Result<Viewer> {
    let args = Args::parse_from(["sims-room", "--paused", "--original", fixture::HOUSE]);
    let (mut viewer, task) = Viewer::new(args); drop(task);
    assert!(viewer.loaded_input.is_none());
    let index = fixture::index()?;
    viewer.icons = Some(Arc::new(dialog_view::Icons::load(&index)?));
    viewer.loaded_input = account.map(Account::register);
    Ok(viewer)
}
fn ready(index: &Index) -> Result<Arc<Ready>> {
    Ok(Arc::new(Ready { restore_index: Some(index.clone()), session: Mutex::new(None),
        client: Mutex::new(None), device: Mutex::new(None), icons: Arc::new(dialog_view::Icons::load(index)?) }))
}
fn installed_input(viewer: &Viewer) -> &sims_runtime::original_session::player_input::Observation {
    viewer.session.as_ref().unwrap().loaded_input_origin().unwrap()
}
fn saved_owners(viewer: &Viewer) -> Result<Vec<u8>> {
    let session = viewer.session.as_ref().context("test owner absent")?;
    Ok(serde_json::to_vec(&json!({"session":session.save_managed()?,
        "dialog_client":viewer.client.as_ref().context("test client absent")?.save_managed(session)?,
        "camera_device":viewer.device.as_ref().map(|d| d.save(session)).transpose()?,
        "application":viewer.application,"floor":viewer.floor,"zoom":viewer.zoom,
        "resident":viewer.resident,"selected":viewer.selected,"tutorial_highlight":viewer.tutorial_highlight,
        "viewport_enter_requested":viewer.viewport_enter_requested,"move_inputs":viewer.move_inputs}))?)
}
#[test]
fn raw_whitespace_same_house_different_save_and_copied_inode_are_distinct_claims() -> Result<()> { bounded_control(raw_whitespace_same_house_different_save_and_copied_inode_are_distinct_claims_body) }
fn raw_whitespace_same_house_different_save_and_copied_inode_are_distinct_claims_body() -> Result<()> {
    let (index, value) = envelope()?;
    let first = InputFile::value(&value)?;
    let mut whitespace = first.raw.clone(); whitespace.extend_from_slice(b" \n\t");
    let second = InputFile::new(whitespace)?;
    let copied = InputFile::new(first.raw.clone())?;
    let mut changed = value.clone(); changed["zoom"] = 0.75.into();
    let changed = InputFile::value(&changed)?;
    let observed = Input::read(&first.path)?;
    assert_eq!(observed.raw_bytes(), first.raw);
    assert_eq!(observed.observation().sha256, engine::digest(&first.raw));
    assert_eq!(observed.observation().byte_count, first.raw.len() as u64);
    let b = Input::read(&second.path)?; let c = Input::read(&copied.path)?; let d = Input::read(&changed.path)?;
    assert_eq!(observed.envelope(), b.envelope());
    assert_ne!(observed.observation().sha256, b.observation().sha256);
    assert_eq!(observed.observation().sha256, c.observation().sha256);
    assert_ne!(observed.observation().sha256, d.observation().sha256);
    assert_eq!(observed.saved().source, d.saved().source);
    #[cfg(unix)] assert_ne!(observed.observation().descriptor_before.as_ref().unwrap().inode,
        c.observation().descriptor_before.as_ref().unwrap().inode);
    let (session, decoded) = observed.restore(&index)?;
    assert_eq!(decoded, value);
    assert_eq!(session.loaded_input_origin().unwrap().sha256, engine::digest(&first.raw));
    Ok(())
}
#[test]
fn bare_typed_saved_exploration_checkpoint_and_serialized_allegations_cannot_certify() -> Result<()> { bounded_control(bare_typed_saved_exploration_checkpoint_and_serialized_allegations_cannot_certify_body) }
fn bare_typed_saved_exploration_checkpoint_and_serialized_allegations_cannot_certify_body() -> Result<()> {
    let (index, mut value) = envelope()?;
    let typed: Saved = serde_json::from_value(value["session"].clone())?;
    let restored = Session::restore(&index, typed.clone())?;
    assert!(restored.loaded_input_origin().is_none());
    assert!(restored.fork_for_exploration()?.loaded_input_origin().is_none());
    let bare = InputFile::new(serde_json::to_vec(&typed)?)?;
    assert!(Input::read(&bare.path).is_err());
    let checkpoint = InputFile::value(&json!({"schema":1,"session":typed,"dialog_client":value["dialog_client"],"application":{}}))?;
    assert!(Input::read(&checkpoint.path).is_err());
    value["loaded_input"] = json!({"sha256":"serialized lie","installed":true});
    value["session"]["loaded_input_origin"] = json!({"sha256":"another lie"});
    let actual = InputFile::value(&value)?;
    let (session, _) = Input::read(&actual.path)?.restore(&index)?;
    assert_eq!(session.loaded_input_origin().unwrap().sha256, engine::digest(&actual.raw));
    assert_ne!(session.loaded_input_origin().unwrap().sha256, "serialized lie");
    let saved = serde_json::to_value(session.save_managed()?)?;
    assert!(saved.get("loaded_input_origin").is_none());
    assert!(Session::restore(&index, serde_json::from_value(saved)?)?.loaded_input_origin().is_none());
    Ok(())
}
#[test]
fn actual_strict_mismatch_is_not_admitted_by_a_matching_raw_hash() -> Result<()> { bounded_control(actual_strict_mismatch_is_not_admitted_by_a_matching_raw_hash_body) }
fn actual_strict_mismatch_is_not_admitted_by_a_matching_raw_hash_body() -> Result<()> {
    let (index, mut value) = envelope()?;
    // Use the actual serialized enum spelling rather than a fabricated string.
    value["session"]["options"]["routing"] = serde_json::to_value(sims_sim::lot_route::path::Policy::InferredRectangles)?;
    let file = InputFile::value(&value)?;
    let input = Input::read(&file.path)?;
    assert_eq!(input.observation().sha256, engine::digest(&file.raw));
    let error = input.restore(&index).err().context("Strict mismatch was accepted")?;
    assert!(format!("{error:#}").contains("routing"));
    Ok(())
}
#[test]
fn malformed_precommit_fields_leave_all_saved_owners_and_old_origin_intact() -> Result<()> { bounded_control(malformed_precommit_fields_leave_all_saved_owners_and_old_origin_intact_body) }
fn malformed_precommit_fields_leave_all_saved_owners_and_old_origin_intact_body() -> Result<()> {
    let (index, value) = envelope()?; let first = InputFile::value(&value)?;
    for field in ["house", "restore", "dialog_client", "camera_device", "camera_device_version", "floor", "resident", "selected", "tutorial_highlight", "viewport_enter_requested", "move_inputs", "application_capture"] {
        let account = Account::default(); let mut owner = viewer(Some(&account))?;
        owner.load_with_index(&first.path, &index)?;
        let before = saved_owners(&owner)?; let origin = installed_input(&owner).clone();
        let ordinal = account.inspect_for_test()["transition_ordinal"].clone();
        let mut malformed = value.clone();
        match field {
            "house" => malformed["session"]["source"] = json!("UserData/Houses/House99.iff"),
            "restore" => malformed["session"]["version"] = 999.into(),
            "dialog_client" => malformed["dialog_client"]["version"] = 999.into(),
            "camera_device" => malformed["camera_device"] = json!({}),
            "camera_device_version" => {
                let session = Session::restore(&index, serde_json::from_value(value["session"].clone())?)?;
                malformed["camera_device"] = serde_json::to_value(fixture::device(&session)?.save(&session)?)?;
                malformed["camera_device"]["version"] = 999.into();
            },
            "tutorial_highlight" => malformed["tutorial_highlight"] = 999.into(),
            "floor" => malformed["floor"] = 9.into(),
            "resident" => malformed["resident"] = 999.into(),
            "selected" => malformed["selected"] = 999.into(),
            "viewport_enter_requested" => malformed[field] = json!("invalid bool"),
            "move_inputs" => malformed[field] = json!("invalid queue"),
            "application_capture" => owner.loaded_input_test_fault = Some("application_capture"),
            _ => unreachable!(),
        }
        let file = InputFile::value(&malformed)?;
        assert!(owner.load_with_index(&file.path, &index).is_err(), "{field}");
        assert_eq!(saved_owners(&owner)?, before, "corrected precommit state for {field}");
        assert_eq!(installed_input(&owner), &origin);
        assert_eq!(account.inspect_for_test()["transition_ordinal"], ordinal);
        assert_eq!(owner.loaded_input_capture()["eligible"], false);
    }
    Ok(())
}
#[test]
fn cold_ready_detach_and_player_commit_are_separate_even_when_outer_install_fails() -> Result<()> { bounded_control(cold_ready_detach_and_player_commit_are_separate_even_when_outer_install_fails_body) }
fn cold_ready_detach_and_player_commit_are_separate_even_when_outer_install_fails_body() -> Result<()> {
    let (index, value) = envelope()?; let first = InputFile::value(&value)?;
    for failure in ["at_tick_limit", "initial_advance", "install_final_refresh"] {
        let account = Account::default(); let mut owner = viewer(Some(&account))?;
        owner.load_with_index(&first.path, &index)?;
        let mut raw = first.raw.clone(); raw.extend_from_slice(b" \n");
        let next = InputFile::new(raw)?; owner.args.load = Some(next.path.clone());
        owner.args.at_tick = if failure == "at_tick_limit" { 1_000_001 } else if failure == "initial_advance" { 1 } else { 0 };
        owner.loaded_input_test_fault = if failure == "at_tick_limit" { None } else { Some(failure) };
        let error = owner.install(ready(&index)?).err().context("expected install failure")?;
        assert!(format!("{error:#}").contains(if failure == "at_tick_limit" { "initial advance exceeds limit" } else { failure }));
        assert_eq!(installed_input(&owner).sha256, engine::digest(&next.raw));
        assert_ne!(installed_input(&owner).sha256, engine::digest(&first.raw));
        let snapshot = account.inspect_for_test();
        assert_eq!(snapshot["transition_ordinal"], 3); // initial load, Ready detach, new load
        assert_eq!(snapshot["accounted_slot"]["install"], "failed");
        assert_eq!(snapshot["eligible"], false);
    }
    Ok(())
}
#[test]
fn cold_additional_common_prepare_and_ui_load_tail_retain_the_actual_failed_new_owner() -> Result<()> { bounded_control(cold_additional_common_prepare_and_ui_load_tail_retain_the_actual_failed_new_owner_body) }
fn cold_additional_common_prepare_and_ui_load_tail_retain_the_actual_failed_new_owner_body() -> Result<()> {
    let (index, value) = envelope()?; let first = InputFile::value(&value)?;
    for stage in ["cold_common_prepare", "ui_load_common_prepare"] {
        let account = Account::default(); let mut owner = viewer(Some(&account))?;
        owner.load_with_index(&first.path, &index)?;
        let mut raw = first.raw.clone(); raw.extend_from_slice(b"\n"); let next = InputFile::new(raw)?;
        owner.loaded_input_test_fault = Some(stage);
        let message = if stage == "cold_common_prepare" {
            owner.args.load = Some(next.path.clone()); Message::Loaded(Ok(ready(&index)?))
        } else { owner.args.save = next.path.clone(); Message::Load };
        drop(owner.update(message));
        // This requires the real managed fixture to reach the injected boundary;
        // an earlier refresh/asset failure does not satisfy the control.
        assert!(owner.error.as_deref().is_some_and(|error| error.contains(stage)), "{:?}", owner.error);
        assert_eq!(installed_input(&owner).sha256, engine::digest(&next.raw));
        assert_eq!(account.inspect_for_test()["accounted_slot"]["install"], "failed");
        assert_eq!(owner.loaded_input_capture()["eligible"], false);
    }
    Ok(())
}
#[test]
fn away_and_back_without_draws_restart_and_fresh_construction_do_not_merge_transitions() -> Result<()> { bounded_control(away_and_back_without_draws_restart_and_fresh_construction_do_not_merge_transitions_body) }
fn away_and_back_without_draws_restart_and_fresh_construction_do_not_merge_transitions_body() -> Result<()> {
    let (index, value) = envelope()?; let first = InputFile::value(&value)?;
    let mut alternate = value.clone(); alternate["zoom"] = 0.75.into(); let other = InputFile::value(&alternate)?;
    let account = Account::default(); let mut owner = viewer(Some(&account))?;
    owner.load_with_index(&first.path, &index)?;
    let old_publication = owner.loaded_input_capture();
    owner.load_with_index(&other.path, &index)?; owner.load_with_index(&first.path, &index)?;
    assert_eq!(account.inspect_for_test()["transition_ordinal"], 3);
    assert_eq!(old_publication["transition_ordinal"], 1);
    assert_eq!(installed_input(&owner).sha256, engine::digest(&first.raw));
    drop(owner.update(Message::Restart));
    assert!(owner.session.is_none());
    assert_eq!(account.inspect_for_test()["transition_ordinal"], 4);
    assert_eq!(account.inspect_for_test()["accounted_slot"]["input"], Value::Null);
    assert!(fixture::session(&index)?.loaded_input_origin().is_none());
    drop(owner); let recreated = viewer(Some(&account))?;
    assert_eq!(account.inspect_for_test()["viewer_lifetime_ordinal"], 2);
    assert_eq!(account.inspect_for_test()["transition_ordinal"], 4);
    assert!(recreated.session.is_none());
    Ok(())
}
#[test]
fn full_saved_state_and_real_step_trace_match_with_recording_disabled_and_enabled() -> Result<()> { bounded_control(full_saved_state_and_real_step_trace_match_with_recording_disabled_and_enabled_body) }
fn full_saved_state_and_real_step_trace_match_with_recording_disabled_and_enabled_body() -> Result<()> {
    let (index, value) = envelope()?; let file = InputFile::value(&value)?;
    let account = Account::default(); let mut disabled = viewer(None)?; let mut enabled = viewer(Some(&account))?;
    disabled.load_with_index(&file.path, &index)?; enabled.load_with_index(&file.path, &index)?;
    assert_eq!(saved_owners(&disabled)?, saved_owners(&enabled)?);
    let before = installed_input(&enabled).clone();
    let mut trace_events = 0usize;
    for _ in 0..3 {
        let mut a = vec![]; let mut b = vec![];
        let left = disabled.session.as_mut().unwrap().step_with_trace(&mut a);
        let scope = enabled.owner_work();
        let right = enabled.session.as_mut().unwrap().step_with_trace(&mut b);
        assert_eq!(format!("{left:?}"), format!("{right:?}"));
        assert_eq!(serde_json::to_value(&a)?, serde_json::to_value(&b)?);
        if let Some(scope) = scope { scope.complete(right.is_ok()); }
        left.context("disabled valid-input step must succeed")?;
        right.context("enabled valid-input step must succeed")?;
        trace_events += a.len();
        assert_eq!(saved_owners(&disabled)?, saved_owners(&enabled)?);
    }
    assert!(trace_events > 0, "valid-input equivalence must exercise real VM traces");
    // Save captures OUTPUT only; typed restoration of that output has no origin.
    let saved = enabled.session.as_ref().unwrap().save_managed()?;
    assert_eq!(installed_input(&enabled), &before);
    assert_eq!(serde_json::to_value(&saved)?, serde_json::to_value(disabled.session.as_ref().unwrap().save_managed()?)?);
    assert!(Session::restore(&index, saved)?.loaded_input_origin().is_none());
    Ok(())
}

#[test]
fn real_service_capture_views_and_exploration_forks_do_not_inherit_observed_origin() -> Result<()> { bounded_control(real_service_capture_views_and_exploration_forks_do_not_inherit_observed_origin_body) }
fn real_service_capture_views_and_exploration_forks_do_not_inherit_observed_origin_body() -> Result<()> {
    let (index, value) = envelope()?; let file = InputFile::value(&value)?;
    let (mut session, _) = Input::read(&file.path)?.restore(&index)?;
    let origin = session.loaded_input_origin().unwrap().clone();
    assert!(session.fork_for_exploration()?.loaded_input_origin().is_none());
    let mut cuts = 0;
    let _outcome = session.step_with_service_observer(&mut |_, observation| {
        cuts += 1; assert!(observation.loaded_input_origin().is_none()); Ok(())
    });
    // A callback must actually run; failure before the real capture is not coverage.
    assert!(cuts > 0, "authored fixture did not reach a real service capture");
    assert_eq!(session.loaded_input_origin(), Some(&origin));
    Ok(())
}

#[test]
fn changing_the_path_after_the_owned_read_cannot_substitute_saved_or_digest() -> Result<()> { bounded_control(changing_the_path_after_the_owned_read_cannot_substitute_saved_or_digest_body) }
fn changing_the_path_after_the_owned_read_cannot_substitute_saved_or_digest_body() -> Result<()> {
    let (index, first) = envelope()?; let file = InputFile::value(&first)?;
    let input = Input::read(&file.path)?;
    let mut later = first.clone(); later["zoom"] = 0.75.into();
    std::fs::write(&file.path, serde_json::to_vec(&later)?)?;
    let (session, decoded) = input.restore(&index)?;
    assert_eq!(decoded, first);
    assert_eq!(session.loaded_input_origin().unwrap().sha256, engine::digest(&file.raw));
    assert_ne!(Input::read(&file.path)?.observation().sha256, session.loaded_input_origin().unwrap().sha256);
    Ok(())
}
#[test]
fn historical_missing_optional_player_fields_keep_their_existing_defaults() -> Result<()> { bounded_control(historical_missing_optional_player_fields_keep_their_existing_defaults_body) }
fn historical_missing_optional_player_fields_keep_their_existing_defaults_body() -> Result<()> {
    let (index, mut value) = envelope()?;
    for key in ["selected", "tutorial_highlight", "viewport_enter_requested", "move_inputs"] {
        value.as_object_mut().unwrap().remove(key);
    }
    let file = InputFile::value(&value)?; let mut owner = viewer(None)?;
    owner.load_with_index(&file.path, &index)?;
    assert!(owner.selected.is_none() && owner.tutorial_highlight.is_none());
    assert!(!owner.viewport_enter_requested && owner.move_inputs.is_empty());
    Ok(())
}
#[test]
fn actual_player_save_writes_no_origin_and_does_not_replace_the_input_owner() -> Result<()> { bounded_control(actual_player_save_writes_no_origin_and_does_not_replace_the_input_owner_body) }
fn actual_player_save_writes_no_origin_and_does_not_replace_the_input_owner_body() -> Result<()> {
    let (index, value) = envelope()?; let file = InputFile::value(&value)?;
    let account = Account::default(); let mut owner = viewer(Some(&account))?;
    owner.load_with_index(&file.path, &index)?;
    owner.icons = Some(Arc::new(dialog_view::Icons::load(&index)?));
    owner.art = owner.icons.as_ref().map(|icons| Arc::new(icons.publish_art()));
    // Preserve the valid fallback-renderer mode restored from this envelope.
    // A frozen device without a presenter is not a valid Save fixture.
    assert!(owner.device.is_none());
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_nanos();
    let directory = std::env::temp_dir().join(format!("authored-player-save-{}-{stamp}", std::process::id()));
    std::fs::create_dir(&directory)?; let game = directory.join("empty-game-root"); std::fs::create_dir(&game)?;
    owner.args.root = game.clone(); owner.args.save = directory.join("output.json");
    let origin = installed_input(&owner).clone(); let ordinal = account.inspect_for_test()["transition_ordinal"].clone();
    let mut disabled = viewer(None)?; disabled.load_with_index(&file.path, &index)?;
    disabled.icons = Some(Arc::new(dialog_view::Icons::load(&index)?));
    disabled.art = disabled.icons.as_ref().map(|icons| Arc::new(icons.publish_art()));
    assert!(disabled.device.is_none());
    disabled.args.root = game.clone(); disabled.args.save = directory.join("disabled-output.json");
    disabled.save()?;
    let scope = owner.owner_work(); let result = owner.save();
    if let Some(scope) = scope { scope.complete(result.is_ok()); } result?;
    assert_eq!(installed_input(&owner), &origin);
    assert_eq!(account.inspect_for_test()["transition_ordinal"], ordinal);
    let output_bytes = std::fs::read(&owner.args.save)?;
    assert_eq!(output_bytes, std::fs::read(&disabled.args.save)?);
    let output: Value = serde_json::from_slice(&output_bytes)?;
    assert!(output.get("loaded_input").is_none());
    assert!(output["session"].get("loaded_input_origin").is_none());
    assert!(Session::restore(&index, serde_json::from_value(output["session"].clone())?)?.loaded_input_origin().is_none());
    std::fs::remove_file(&owner.args.save)?; std::fs::remove_file(&disabled.args.save)?; std::fs::remove_dir(game)?; std::fs::remove_dir(directory)?;
    Ok(())
}

#[test]
fn installation_success_requires_the_real_cold_or_ui_common_tail() -> Result<()> { bounded_control(installation_success_requires_the_real_cold_or_ui_common_tail_body) }
fn installation_success_requires_the_real_cold_or_ui_common_tail_body() -> Result<()> {
    let (index, value) = envelope()?; let file = InputFile::value(&value)?;
    for cold in [false, true] {
        let account = Account::default(); let mut owner = viewer(Some(&account))?;
        if cold {
            owner.args.load = Some(file.path.clone());
            drop(owner.update(Message::Loaded(Ok(ready(&index)?))));
        } else {
            owner.load_with_index(&file.path, &index)?;
            let before = owner.loaded_input_capture();
            assert_eq!(before["accounted_slot"]["install"], "pending");
            assert_eq!(before["eligible"], false);
            owner.args.save = file.path.clone(); drop(owner.update(Message::Load));
            assert_eq!(before["accounted_slot"]["install"], "pending");
        }
        assert!(owner.error.is_none(), "real preparation must succeed: {:?}", owner.error);
        assert_eq!(account.inspect_for_test()["accounted_slot"]["install"], "installed");
        assert_eq!(installed_input(&owner).sha256, engine::digest(&file.raw));
    }
    Ok(())
}

#[test]
fn real_loaded_owner_worker_failure_keeps_old_immutable_frame_and_new_failed_owner() -> Result<()> { bounded_control(real_loaded_owner_worker_failure_keeps_old_immutable_frame_and_new_failed_owner_body) }
fn real_loaded_owner_worker_failure_keeps_old_immutable_frame_and_new_failed_owner_body() -> Result<()> {
    let (index, value) = envelope()?; let first = InputFile::value(&value)?;
    let mut raw = first.raw.clone(); raw.extend_from_slice(b" \n"); let next = InputFile::new(raw)?;
    let account = Account::default(); let mut owner = viewer(Some(&account))?;
    owner.load_with_index(&first.path, &index)?;
    owner.args.save = first.path.clone(); drop(owner.update(Message::Load));
    assert!(owner.error.is_none(), "initial real common preparation must succeed: {:?}", owner.error);
    owner.args.save = next.path.clone(); owner.loaded_input_test_fault = Some("post_update_popup_counter");
    let mut host = worker_host::Host::from_viewer_for_test(owner);
    let before = host.loaded_input_publication_for_test().context("captured input missing")?;
    drop(host.update(worker_host::Event::Widget { generation: 0, message: Message::Load }));
    assert!(host.settle_for_test().is_err());
    assert_eq!(host.loaded_input_publication_for_test().unwrap(), before);
    let owner = host.recover_for_test();
    assert!(owner.error.is_none(), "new loader/common tail must really have succeeded before worker failure");
    assert_eq!(installed_input(&owner).sha256, engine::digest(&next.raw));
    let snapshot = account.terminal();
    assert_eq!(snapshot["accounted_slot"]["input"]["sha256"], engine::digest(&next.raw));
    assert_eq!(snapshot["accounted_slot"]["install"], "failed");
    assert_ne!(snapshot["transition_ordinal"], before["transition_ordinal"]);
    assert_eq!(snapshot["eligible"], false);
    assert_eq!(before["accounted_slot"]["input"]["sha256"], engine::digest(&first.raw));
    assert_eq!(snapshot["worker_finalized"], false);
    Ok(())
}

fn prepared_player(account: &Account, index: &Index, file: &InputFile) -> Result<Viewer> {
    let mut owner = viewer(Some(account))?;
    owner.load_with_index(&file.path, index)?;
    owner.args.save = file.path.clone(); drop(owner.update(Message::Load));
    ensure!(owner.error.is_none(), "real common preparation failed: {:?}", owner.error);
    let capture = owner.loaded_input_capture();
    assert_eq!(capture["eligible"], true);
    assert_eq!(capture["capture_boundary"], "completed_owner_work");
    assert_eq!(capture["accounted_slot"]["input"]["sha256"], engine::digest(&file.raw));
    Ok(owner)
}
fn assert_prepared_publication(input: &Value, file: &InputFile) {
    assert_eq!(input["extension"], "sims-window-loaded-input-observation-v2");
    assert_eq!(input["eligible"], true);
    assert_eq!(input["capture_owner_matches_account"], true);
    assert_eq!(input["current_owner_asserted"], true);
    assert_eq!(input["accounted_slot"]["install"], "installed");
    assert_eq!(input["accounted_slot"]["input"]["sha256"], engine::digest(&file.raw));
    assert_eq!(input["accounted_slot"]["input"]["byte_count"], file.raw.len() as u64);
    assert_eq!(input["accounted_slot"]["input"]["descriptor_status"], "consistent");
    assert_eq!(input["capture_boundary"], "owner_prepared_before_read_only_publication");
    assert_eq!(input["owner_preparation_complete"], true);
    assert_eq!(input["owner_mutation_pending"], false);
    assert_eq!(input["publication_delivery_pending"], true);
    assert_eq!(input["unresolved_in_flight"], true);
    assert_eq!(input["ownership_status"], "owner_prepared_delivery_pending");
    assert_eq!(input["last_operation"], "owner_prepared");
    assert_eq!(input["active_work"].as_array().unwrap().len(), 1);
    assert!(input["invalidation_reasons"].as_array().unwrap().is_empty());
    assert_eq!(input["worker_finalized"], false); assert_eq!(input["quiescence_proven"], false);
}

#[test]
fn real_host_cold_and_ui_load_publish_eligible_exact_input_and_retain_old_frame_while_loading() -> Result<()> {
    bounded_control(real_host_cold_and_ui_load_publish_eligible_exact_input_and_retain_old_frame_while_loading_body)
}
fn real_host_cold_and_ui_load_publish_eligible_exact_input_and_retain_old_frame_while_loading_body() -> Result<()> {
    let (index, value) = envelope()?; let first = InputFile::value(&value)?;
    let mut raw = first.raw.clone(); raw.extend_from_slice(b" \n\t"); let next = InputFile::new(raw)?;
    let account = Account::default(); let mut owner = viewer(Some(&account))?;
    owner.args.load = Some(first.path.clone()); owner.args.save = next.path.clone();
    let mut host = worker_host::Host::from_viewer_for_test(owner);
    assert_eq!(host.loaded_input_publication_for_test().unwrap()["eligible"], false);
    drop(host.update(worker_host::Event::Effect { generation: 0, message: Message::Loaded(Ok(ready(&index)?)) }));
    host.settle_for_test()?;
    let before = host.loaded_input_publication_for_test().context("cold loaded input missing")?;
    assert_prepared_publication(&before, &first);
    assert_eq!(before["transition_ordinal"], 1);
    let old = host.publication_for_test().clone();
    let (entered, resume) = host.gate_next_for_test();
    let event = host.ui_event_for_test(&old, Message::Load); drop(host.update(event));
    entered.recv_timeout(std::time::Duration::from_secs(5))?;
    assert!(Arc::ptr_eq(&old, host.publication_for_test()));
    assert_eq!(host.loaded_input_publication_for_test().unwrap(), before);
    let in_flight = account.inspect_for_test();
    assert_eq!(in_flight["owner_mutation_pending"], true);
    assert_eq!(in_flight["owner_preparation_complete"], false);
    assert_eq!(in_flight["eligible"], false);
    resume.send(())?; host.settle_for_test()?;
    let after = host.loaded_input_publication_for_test().context("UI loaded input missing")?;
    assert_prepared_publication(&after, &next);
    assert_eq!(after["transition_ordinal"], 2);
    assert!(!Arc::ptr_eq(&old, host.publication_for_test()));
    assert_eq!(old.window_observation.as_ref().unwrap()["loaded_input"], before);
    assert_ne!(after["accounted_slot"]["input"]["sha256"], before["accounted_slot"]["input"]["sha256"]);
    let owner = host.recover_for_test();
    assert_eq!(installed_input(&owner).sha256, engine::digest(&next.raw));
    // Unknown test recovery is deliberately not the UI's retirement protocol.
    assert!(account.inspect_for_test()["invalidation_reasons"].as_array().unwrap().iter().any(|v| v == "request_channel_disconnected"));
    assert_eq!(old.window_observation.as_ref().unwrap()["loaded_input"], before);
    Ok(())
}

#[test]
fn real_host_absorbed_preparation_popup_publication_and_panics_keep_actual_failed_owner() -> Result<()> {
    bounded_control(real_host_absorbed_preparation_popup_publication_and_panics_keep_actual_failed_owner_body)
}
fn real_host_absorbed_preparation_popup_publication_and_panics_keep_actual_failed_owner_body() -> Result<()> {
    let (index, value) = envelope()?; let first = InputFile::value(&value)?;
    let mut raw = first.raw.clone(); raw.extend_from_slice(b" \n"); let next = InputFile::new(raw)?;
    for fault in ["ui_load_common_prepare", "post_update_popup_counter", "worker_publication", "worker_apply_panic", "worker_publication_panic"] {
        let account = Account::default(); let mut owner = prepared_player(&account, &index, &first)?;
        owner.args.save = next.path.clone(); owner.loaded_input_test_fault = Some(fault);
        let mut host = worker_host::Host::from_viewer_for_test(owner);
        let old = host.publication_for_test().clone();
        let before = host.loaded_input_publication_for_test().unwrap(); assert_prepared_publication(&before, &first);
        drop(host.update(worker_host::Event::Widget { generation: 0, message: Message::Load }));
        if fault.ends_with("panic") {
            assert!(host.recover_panicked_for_test(), "{fault}");
        } else {
            let outcome = host.settle_for_test();
            if fault == "ui_load_common_prepare" {
                outcome?;
                let failed_frame = host.loaded_input_publication_for_test().unwrap();
                assert_eq!(failed_frame["accounted_slot"]["input"]["sha256"], engine::digest(&next.raw));
                assert_eq!(failed_frame["eligible"], false);
            } else {
                assert!(outcome.is_err(), "{fault}");
                assert!(Arc::ptr_eq(&old, host.publication_for_test()), "{fault}");
                assert_eq!(host.loaded_input_publication_for_test().unwrap(), before);
            }
            let owner = host.recover_for_test();
            assert_eq!(installed_input(&owner).sha256, engine::digest(&next.raw));
            if fault == "ui_load_common_prepare" {
                assert!(owner.error.as_deref().is_some_and(|error| error.contains(fault)));
            } else { assert!(owner.error.is_none()); }
            drop(owner);
        }
        assert_eq!(old.window_observation.as_ref().unwrap()["loaded_input"], before);
        let footer = account.terminal();
        assert_eq!(footer["accounted_slot"]["input"]["sha256"], engine::digest(&next.raw), "{fault}");
        assert_eq!(footer["last_recorded_owner"]["input"]["sha256"], engine::digest(&next.raw), "{fault}");
        assert_eq!(footer["accounted_slot"]["install"], "failed", "{fault}");
        assert_eq!(footer["eligible"], false); assert_eq!(footer["worker_finalized"], false);
        assert!(footer["invalidation_reasons"].as_array().unwrap().iter().any(|v| v == "owner_work_failed"), "{fault}");
        if fault.ends_with("panic") {
            assert!(footer["invalidation_reasons"].as_array().unwrap().iter().any(|v| v == "owner_panicked"), "{fault}");
        }
    }
    Ok(())
}

#[test]
fn real_host_send_failure_and_notification_panic_never_retrofit_captured_input() -> Result<()> {
    bounded_control(real_host_send_failure_and_notification_panic_never_retrofit_captured_input_body)
}
fn real_host_send_failure_and_notification_panic_never_retrofit_captured_input_body() -> Result<()> {
    let (index, value) = envelope()?; let first = InputFile::value(&value)?;
    let mut raw = first.raw.clone(); raw.extend_from_slice(b"\n"); let next = InputFile::new(raw)?;
    for notify_panic in [false, true] {
        let account = Account::default(); let mut owner = prepared_player(&account, &index, &first)?;
        owner.args.save = next.path.clone(); let mut host = worker_host::Host::from_viewer_for_test(owner);
        let old = host.publication_for_test().clone(); let before = host.loaded_input_publication_for_test().unwrap();
        assert_prepared_publication(&before, &first);
        if notify_panic {
            host.notification_for_test(Box::new(|| panic!("authored real owner notification panic")));
            drop(host.update(worker_host::Event::Widget { generation: 0, message: Message::Load }));
            assert!(host.recover_panicked_for_test());
        } else {
            let (entered, resume) = host.gate_next_for_test();
            drop(host.update(worker_host::Event::Widget { generation: 0, message: Message::Load }));
            entered.recv_timeout(std::time::Duration::from_secs(5))?;
            host.disconnect_results_for_test(); resume.send(())?;
            assert!(Arc::ptr_eq(&old, host.publication_for_test()));
            let owner = host.recover_for_test();
            assert_eq!(installed_input(&owner).sha256, engine::digest(&next.raw)); drop(owner);
        }
        assert_eq!(old.window_observation.as_ref().unwrap()["loaded_input"], before);
        let footer = account.terminal();
        assert_eq!(footer["accounted_slot"]["input"]["sha256"], engine::digest(&next.raw));
        assert_eq!(footer["last_recorded_owner"]["input"]["sha256"], engine::digest(&next.raw));
        assert_eq!(footer["accounted_slot"]["install"], "failed");
        let reason = if notify_panic { "abandoned_or_panicked_work" } else { "result_delivery_failed" };
        assert!(footer["invalidation_reasons"].as_array().unwrap().iter().any(|v| v == reason));
        assert_eq!(footer["request_channel_closed_after_ui_retirement"], false);
        assert_eq!(footer["eligible"], false); assert_eq!(footer["worker_finalized"], false);
    }
    Ok(())
}

#[test]
fn real_host_ui_can_close_during_notification_but_early_footer_keeps_open_guard_forever() -> Result<()> {
    bounded_control(real_host_ui_can_close_during_notification_but_early_footer_keeps_open_guard_forever_body)
}
fn real_host_ui_can_close_during_notification_but_early_footer_keeps_open_guard_forever_body() -> Result<()> {
    let (index, value) = envelope()?; let first = InputFile::value(&value)?;
    let mut raw = first.raw.clone(); raw.extend_from_slice(b"\n "); let next = InputFile::new(raw)?;
    let account = Account::default(); let mut owner = prepared_player(&account, &index, &first)?;
    owner.args.save = next.path.clone(); let mut host = worker_host::Host::from_viewer_for_test(owner);
    let (entered, ready) = std::sync::mpsc::sync_channel(1); let (release, resume) = std::sync::mpsc::sync_channel(1);
    host.notification_for_test(Box::new(move || { entered.send(()).unwrap(); resume.recv().unwrap(); }));
    drop(host.update(worker_host::Event::Widget { generation: 0, message: Message::Load }));
    ready.recv_timeout(std::time::Duration::from_secs(5))?;
    host.settle_for_test()?; assert!(host.frontend_idle_for_test());
    let captured = host.loaded_input_publication_for_test().unwrap(); assert_prepared_publication(&captured, &next);
    assert_eq!(host.update(worker_host::Event::Close(window::Id::unique())).units(), 1);
    let footer = account.terminal();
    assert_eq!(footer["ui_retirement_intent_lifetime"], 1);
    assert_eq!(footer["request_channel_closed_after_ui_retirement"], false);
    assert_eq!(footer["retired_lifetime"], Value::Null);
    assert_eq!(footer["unresolved_in_flight"], true);
    assert_eq!(footer["owner_preparation_complete"], true);
    assert_eq!(footer["publication_delivery_pending"], true);
    assert_eq!(footer["owner_mutation_pending"], false);
    assert_eq!(footer["active_work"].as_array().unwrap().len(), 1);
    assert_eq!(footer["snapshot_only"], true); assert_eq!(footer["eligible"], false);
    assert_eq!(footer["current_owner_asserted"], false);
    assert_eq!(footer["worker_finalized"], false); assert_eq!(footer["quiescence_proven"], false);
    assert_eq!(host.loaded_input_publication_for_test().unwrap(), captured);
    release.send(())?; let owner = host.recover_for_test(); drop(owner);
    assert_eq!(account.terminal(), footer);
    assert_eq!(captured["eligible"], true);
    Ok(())
}

#[test]
fn real_host_close_drains_accepted_work_and_records_only_observed_retirement() -> Result<()> {
    bounded_control(real_host_close_drains_accepted_work_and_records_only_observed_retirement_body)
}
fn real_host_close_drains_accepted_work_and_records_only_observed_retirement_body() -> Result<()> {
    let (index, value) = envelope()?; let file = InputFile::value(&value)?;
    let account = Account::default(); let owner = prepared_player(&account, &index, &file)?;
    let mut host = worker_host::Host::from_viewer_for_test(owner);
    let (entered, resume) = host.gate_next_for_test();
    drop(host.update(worker_host::Event::Widget { generation: 0, message: Message::Shift(true) }));
    entered.recv_timeout(std::time::Duration::from_secs(5))?;
    drop(host.update(worker_host::Event::Widget { generation: 0, message: Message::Shift(false) }));
    drop(host.update(worker_host::Event::Widget { generation: 0, message: Message::FocusLost }));
    let id = window::Id::unique(); assert_eq!(host.update(worker_host::Event::Close(id)).units(), 0);
    assert_eq!(account.inspect_for_test()["ui_retirement_intent_lifetime"], Value::Null);
    resume.send(())?; host.settle_for_test()?;
    assert_eq!(host.update(worker_host::Event::Completion).units(), 1);
    assert_eq!(host.update(worker_host::Event::Close(id)).units(), 0);
    let owner = host.recover_for_test();
    assert!(!owner.shift && owner.last.is_none());
    assert_eq!(owner.widget_epoch, 1);
    assert_eq!(installed_input(&owner).sha256, engine::digest(&file.raw));
    let closed = account.inspect_for_test();
    assert_eq!(closed["ui_retirement_intent_lifetime"], 1);
    assert_eq!(closed["request_channel_closed_after_ui_retirement"], true);
    assert!(closed["active_work"].as_array().unwrap().is_empty());
    assert!(closed["invalidation_reasons"].as_array().unwrap().is_empty());
    drop(owner); let footer = account.terminal();
    assert_eq!(footer["retired_lifetime"], 1); assert_eq!(footer["live_lifetime"], Value::Null);
    assert_eq!(footer["accounted_slot"]["input"]["sha256"], engine::digest(&file.raw));
    assert_eq!(footer["snapshot_only"], true); assert_eq!(footer["eligible"], false);
    assert_eq!(footer["worker_finalized"], false); assert_eq!(footer["quiescence_proven"], false);
    Ok(())
}

// Component output only: an enclosing journal, draw/receipt and native launch
// are outside this asset-free control. The consumer's enclosing fixture must
// remain explicitly synthetic even when it embeds these actual Rust captures.
#[cfg(unix)]
#[test]
fn real_host_loaded_input_component_serializes_actual_collector_and_retired_terminal() -> Result<()> {
    bounded_control(real_host_loaded_input_component_serializes_actual_collector_and_retired_terminal_body)
}
#[cfg(unix)]
fn real_host_loaded_input_component_serializes_actual_collector_and_retired_terminal_body() -> Result<()> {
    use std::os::unix::fs::MetadataExt;
    fn file_state(file: &InputFile) -> Result<Value> {
        // Independent pathname metadata brackets the loader's own descriptor
        // observations. Hash the owned bytes written by InputFile, never reopen.
        let metadata = std::fs::metadata(&file.path)?;
        ensure!(metadata.is_file() && metadata.nlink() == 1,
            "component input must remain a single-link regular file");
        ensure!(metadata.len() == file.raw.len() as u64,
            "component input length differs from its authored bytes");
        Ok(json!({
            "sha256":engine::digest(&file.raw),"size":metadata.len(),
            "mode":metadata.mode() & 0o7777,"device":metadata.dev(),
            "inode":metadata.ino(),"links":metadata.nlink(),
            "mtime_ns":i128::from(metadata.mtime()) * 1_000_000_000 + i128::from(metadata.mtime_nsec()),
            "ctime_ns":i128::from(metadata.ctime()) * 1_000_000_000 + i128::from(metadata.ctime_nsec())
        }))
    }

    let (index, value) = envelope()?;
    // This is the actual fixture's saved policy; never relabel its raw input
    // or relax the loader's ordinary policy/restore guards to make it qualify.
    assert_eq!(value["session"]["options"]["routing"],
        serde_json::to_value(sims_sim::lot_route::path::Policy::Strict)?);
    let selected = InputFile::value(&value)?;
    let pinned = InputFile::new(selected.raw.clone())?;
    assert_ne!(selected.path, pinned.path);
    assert_eq!(selected.raw, pinned.raw);
    let account = Account::default();
    let mut owner = viewer(Some(&account))?;
    owner.args.load = Some(pinned.path.clone());
    let mut host = worker_host::Host::from_viewer_for_test(owner);
    let selected_before = file_state(&selected)?;
    let pinned_before = file_state(&pinned)?;
    assert!(selected_before["device"] != pinned_before["device"]
        || selected_before["inode"] != pinned_before["inode"]);
    drop(host.update(worker_host::Event::Effect {
        generation: 0, message: Message::Loaded(Ok(ready(&index)?)),
    }));
    host.settle_for_test()?;
    let selected_after = file_state(&selected)?;
    let pinned_after = file_state(&pinned)?;
    assert_eq!(selected_before, selected_after);
    assert_eq!(pinned_before, pinned_after);

    let publication = host.publication_for_test().clone();
    let captured = publication.window_observation.as_ref()
        .context("real cold-load window observation absent")?.as_ref().clone();
    assert_prepared_publication(&captured["loaded_input"], &pinned);
    assert_eq!(captured["loaded_input"]["transition_ordinal"], 1);
    assert_eq!(captured["loaded_input"]["accounted_slot"]["input"]["requested_path"],
        pinned.path.to_string_lossy().as_ref());
    let observed = super::super::super::support::window_session::observed_state_for_test(captured.clone());
    let immutable_observed = observed.as_ref().clone();
    assert_eq!(observed["value"]["loaded_input"]["eligible"], true);
    assert_eq!(observed["value"]["loaded_input"]["accounted_slot"]["input"]["sha256"],
        engine::digest(&pinned.raw));
    assert!(observed["value"]["loaded_input"].get("work_ordinal").is_none());
    assert!(observed["value"]["loaded_input"].get("active_work").is_none());
    for name in ["work_ordinal", "active_work"] {
        assert_eq!(observed["diagnostics"]["loaded_input_work"][name], captured["loaded_input"][name]);
    }

    let id = window::Id::unique();
    assert_eq!(host.update(worker_host::Event::Close(id)).units(), 1);
    assert_eq!(host.update(worker_host::Event::Close(id)).units(), 0);
    // The real UI close protocol precedes this OFF-UI, test-only join. The
    // resulting ledger still makes no worker-finalization/quiescence claim.
    let owner = host.recover_for_test();
    ensure!(owner.error.is_none(), "real cold preparation failed: {:?}", owner.error);
    let session = owner.session.as_ref().context("real loaded session absent")?;
    assert_eq!(session.options.routing, sims_sim::lot_route::path::Policy::Strict);
    assert_eq!(session.simulation.native_lot.as_ref().context("authored lot absent")?.routing_policy,
        sims_sim::lot_route::path::Policy::Strict);
    assert_eq!(installed_input(&owner).sha256, engine::digest(&pinned.raw));
    let source = session.house.source.clone();
    drop(owner);
    let terminal = account.terminal();
    assert_eq!(terminal["ui_retirement_intent_lifetime"], 1);
    assert_eq!(terminal["request_channel_closed_after_ui_retirement"], true);
    assert_eq!(terminal["retired_lifetime"], 1);
    assert_eq!(terminal["live_lifetime"], Value::Null);
    assert_eq!(terminal["closed"], true);
    assert_eq!(terminal["snapshot_only"], true);
    assert!(terminal["active_work"].as_array().unwrap().is_empty());
    assert!(terminal["invalidation_reasons"].as_array().unwrap().is_empty());
    for name in ["eligible", "current_owner_asserted", "unresolved_in_flight",
        "owner_preparation_complete", "owner_mutation_pending", "publication_delivery_pending",
        "worker_finalized", "quiescence_proven"] {
        assert_eq!(terminal[name], false, "{name}");
    }
    assert_eq!(terminal["accounted_slot"]["input"]["sha256"], engine::digest(&pinned.raw));
    assert_eq!(account.terminal(), terminal);
    assert_eq!(observed.as_ref(), &immutable_observed);
    assert_eq!(publication.window_observation.as_ref().unwrap().as_ref(), &captured);
    assert_eq!(observed["value"]["loaded_input"]["eligible"], true);

    println!("SIMS_LOADED_INPUT_COMPONENT_V1={}", serde_json::to_string(&json!({
        "contract":"sims-loaded-input-cross-language-component-v1",
        "scope":"asset-free managed-loader/collector serialization; no native enclosing journal/receipt execution proof",
        "selected":{"path":selected.path.to_string_lossy(),"raw_bytes":&selected.raw,
            "before":selected_before,"after":selected_after},
        "pinned":{"path":pinned.path.to_string_lossy(),"raw_bytes":&pinned.raw,
            "before":pinned_before,"after":pinned_after},
        "source":source,"observed_state":observed.as_ref(),"terminal":terminal
    }))?);
    Ok(())
}
