use super::super::super::player_timing::{self, Operation, Outcome, Recorder, Role, Scope};
use super::super::super::ui_publication::{Intent, JourneyTarget, Published, Resident};
use super::super::super::worker_host::{Event, Host};
use super::*;
use iced::futures::{StreamExt, executor::ThreadPool};
use sims_render::camera_device::panels::ButtonView;
use sims_vm::interface::view_control::Widget;
use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::sync::mpsc::{self, Receiver, Sender};

#[path = "host_journey/tutorial_camera.rs"]
mod tutorial_camera;

const MAX_COMMANDS: usize = 100_000;
const MAX_FRAMES: u32 = 100_000;
const MAX_POLLS: usize = 100_000;
const MAX_TICKS: u64 = 50_000;
const MAX_TARGETS: usize = 32;
const MAX_HIT_TESTS: usize = 2_048;

struct VisibleTargets {
    discovered: Vec<(i16, [f32; 2])>,
    sampled: Vec<(i16, [f32; 2])>,
    candidate_objects: usize,
    candidate_centroids: usize,
    probed_objects: usize,
    probes: usize,
    excluded: TargetExclusions,
    sit_candidates: usize,
    sit_discovered: usize,
    sit_sampled: usize,
}

#[derive(Default)]
struct TargetExclusions {
    people: BTreeMap<i16, i16>,
    missing_metadata: BTreeSet<i16>,
    metadata_errors: BTreeMap<i16, String>,
}

impl TargetExclusions {
    fn priority(
        &mut self,
        object: i16,
        metadata: Option<&std::result::Result<JourneyTarget, String>>,
    ) -> Option<bool> {
        match metadata {
            Some(Ok(JourneyTarget::Object { table_sit, .. })) => Some(*table_sit),
            Some(Ok(JourneyTarget::Person(target))) => {
                self.people.insert(object, *target);
                None
            }
            Some(Err(error)) => {
                self.metadata_errors.insert(object, error.clone());
                None
            }
            None => {
                self.missing_metadata.insert(object);
                None
            }
        }
    }

    fn observation(&self) -> Value {
        json!({"resolved_person_count":self.people.len(),"resolved_person_targets":self.people,
            "missing_metadata_count":self.missing_metadata.len(),
            "missing_metadata_objects":self.missing_metadata,
            "failed_metadata_count":self.metadata_errors.len(),"metadata_errors":self.metadata_errors})
    }
}

fn spread_indices(length: usize, limit: usize) -> Vec<usize> {
    let count = length.min(limit);
    if count < 2 {
        return (0..count).collect();
    }
    (0..count)
        .map(|index| (index as u128 * (length - 1) as u128 / (count - 1) as u128) as usize)
        .collect()
}

fn prioritized_indices(priorities: &[bool], limit: usize) -> Vec<usize> {
    let mut preferred = Vec::new();
    let mut fallback = Vec::new();
    for (index, &priority) in priorities.iter().enumerate() {
        if priority {
            preferred.push(index);
        } else {
            fallback.push(index);
        }
    }
    let mut selected: Vec<_> = spread_indices(preferred.len(), limit)
        .into_iter()
        .map(|index| preferred[index])
        .collect();
    selected.extend(
        spread_indices(fallback.len(), limit - selected.len())
            .into_iter()
            .map(|index| fallback[index]),
    );
    selected
}

fn discovery_probe_allowance(position: usize, objects: usize) -> usize {
    if objects == 0 {
        return 0;
    }
    let limit = MAX_HIT_TESTS - MAX_TARGETS;
    limit / objects + usize::from(position < limit % objects)
}

#[test]
fn visible_target_spread_preserves_small_lists() {
    assert!(spread_indices(0, MAX_TARGETS).is_empty());
    assert!(spread_indices(100, 0).is_empty());
    assert_eq!(spread_indices(100, 1), vec![0]);
    for length in 1..=MAX_TARGETS {
        assert_eq!(
            spread_indices(length, MAX_TARGETS),
            (0..length).collect::<Vec<_>>()
        );
    }
}

#[test]
fn visible_target_spread_includes_both_endpoints() {
    assert_eq!(spread_indices(9, 4), vec![0, 2, 5, 8]);
    for length in [33, 97, 4_096, usize::MAX] {
        let sampled = spread_indices(length, MAX_TARGETS);
        assert_eq!(sampled.len(), MAX_TARGETS);
        assert_eq!(sampled.first(), Some(&0));
        assert_eq!(sampled.last(), Some(&(length - 1)));
    }
}

#[test]
fn visible_target_spread_is_deterministic_ordered_and_unique() {
    for length in 0..=128 {
        for limit in 0..=MAX_TARGETS {
            let sampled = spread_indices(length, limit);
            assert_eq!(sampled, spread_indices(length, limit));
            assert_eq!(sampled.len(), length.min(limit));
            assert!(sampled.iter().all(|index| *index < length));
            assert!(sampled.windows(2).all(|pair| pair[0] < pair[1]));
            assert_eq!(sampled.iter().collect::<BTreeSet<_>>().len(), sampled.len());
        }
    }
}

#[test]
fn furniture_target_eligibility_excludes_people_and_unavailable_metadata() {
    let mut excluded = TargetExclusions::default();
    let visitor = Ok(JourneyTarget::Person(259));
    assert_eq!(excluded.priority(259, Some(&visitor)), None);
    assert_eq!(excluded.priority(173, Some(&visitor)), None);
    assert_eq!(excluded.priority(173, Some(&visitor)), None);
    assert_eq!(excluded.people, BTreeMap::from([(173, 259), (259, 259)]));
    assert_eq!(excluded.priority(174, None), None);
    let error = Err("interaction target definition missing".to_owned());
    assert_eq!(excluded.priority(223, Some(&error)), None);
    let sit = Ok(JourneyTarget::Object {
        target: 174,
        table_sit: true,
    });
    let fallback = Ok(JourneyTarget::Object {
        target: 224,
        table_sit: false,
    });
    assert_eq!(excluded.priority(174, Some(&sit)), Some(true));
    assert_eq!(excluded.priority(224, Some(&fallback)), Some(false));
    let observation = excluded.observation();
    assert_eq!(observation["resolved_person_count"], 2);
    assert_eq!(observation["missing_metadata_count"], 1);
    assert_eq!(observation["failed_metadata_count"], 1);
    assert_eq!(
        observation["metadata_errors"]["223"],
        "interaction target definition missing"
    );
}

#[test]
fn furniture_target_priority_recovers_seats_omitted_by_endpoint_spread() {
    let mut discovered: Vec<i16> = (100..213).collect();
    let seats = [173, 174, 223];
    discovered[76] = 223;
    let old_sample = spread_indices(discovered.len(), MAX_TARGETS);
    assert!(
        seats
            .iter()
            .all(|seat| !old_sample.iter().any(|&index| discovered[index] == *seat))
    );
    let priorities: Vec<_> = discovered
        .iter()
        .map(|object| seats.contains(object))
        .collect();
    let sampled = prioritized_indices(&priorities, MAX_TARGETS);
    assert_eq!(
        sampled
            .iter()
            .take(seats.len())
            .map(|&index| discovered[index])
            .collect::<Vec<_>>(),
        seats
    );
    assert_eq!(sampled.len(), MAX_TARGETS);
    let fallback: Vec<_> = (0..discovered.len())
        .filter(|&index| !priorities[index])
        .collect();
    let expected_fallback: Vec<_> = spread_indices(fallback.len(), MAX_TARGETS - seats.len())
        .iter()
        .map(|&index| fallback[index])
        .collect();
    assert_eq!(&sampled[seats.len()..], expected_fallback.as_slice());
}

#[test]
fn furniture_target_priority_and_probe_allowances_are_deterministic_and_bounded() {
    let discovery_limit = MAX_HIT_TESTS - MAX_TARGETS;
    for length in [0, 1, 31, 32, 33, 113, 2_016, 2_017, 4_096] {
        for priorities in [
            vec![false; length],
            vec![true; length],
            (0..length).map(|index| index % 7 == 1).collect(),
        ] {
            for limit in [0, 1, MAX_TARGETS, discovery_limit] {
                let sampled = prioritized_indices(&priorities, limit);
                assert_eq!(sampled, prioritized_indices(&priorities, limit));
                assert_eq!(sampled.len(), length.min(limit));
                assert_eq!(sampled.iter().collect::<BTreeSet<_>>().len(), sampled.len());
                assert!(sampled.iter().all(|&index| index < length));
                assert_eq!(
                    sampled.iter().filter(|&&index| priorities[index]).count(),
                    priorities
                        .iter()
                        .filter(|&&priority| priority)
                        .count()
                        .min(limit)
                );
                assert!(
                    !sampled
                        .windows(2)
                        .any(|pair| !priorities[pair[0]] && priorities[pair[1]])
                );
                if priorities.iter().all(|&priority| !priority) {
                    assert_eq!(sampled, spread_indices(length, limit));
                }
            }
            let objects = prioritized_indices(&priorities, discovery_limit);
            let allowances: Vec<_> = (0..objects.len())
                .map(|position| discovery_probe_allowance(position, objects.len()))
                .collect();
            let probes: usize = allowances.iter().sum();
            assert!(allowances.iter().all(|&allowance| allowance >= 1));
            assert!(probes <= discovery_limit);
            assert!(probes + prioritized_indices(&priorities, MAX_TARGETS).len() <= MAX_HIT_TESTS);
        }
    }
    assert_eq!(discovery_probe_allowance(0, 0), 0);
}

#[test]
#[ignore = "private assets; fresh SIMS_HOST_JOURNEY_OUTPUT; continuous production Host"]
fn continuous_real_host_player_journey() -> Result<()> {
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(run)?
        .join()
        .expect("continuous Host journey panicked")
}

enum Delivery {
    Action(iced_runtime::Action<Event>),
    Finished,
}

struct Journey {
    host: Host,
    executor: ThreadPool,
    sender: Sender<Delivery>,
    receiver: Receiver<Delivery>,
    streams: usize,
    tasks: usize,
    idle: bool,
    wall: Instant,
    wall_seconds: f64,
    epoch: Instant,
    millis: u64,
    commands: usize,
    frames: u32,
    service_polls: usize,
    effect_polls: usize,
    dialogs: usize,
    events: Vec<Value>,
    timing: Option<Recorder>,
    timing_parent: Option<player_timing::Id>,
}

trait TimingOutcome {
    fn timing_outcome(&self) -> Outcome;
}

impl TimingOutcome for () {
    fn timing_outcome(&self) -> Outcome {
        Outcome::Completed
    }
}

impl TimingOutcome for Value {
    fn timing_outcome(&self) -> Outcome {
        if matches!(self["status"].as_str(), Some("unsupported" | "partial")) {
            Outcome::Unsupported
        } else {
            Outcome::Completed
        }
    }
}

impl<Entry> TimingOutcome for Vec<Entry> {
    fn timing_outcome(&self) -> Outcome {
        Outcome::Completed
    }
}

impl<First, Second> TimingOutcome for (First, Second) {
    fn timing_outcome(&self) -> Outcome {
        Outcome::Completed
    }
}

fn tick(publication: &Published) -> Result<u64> {
    publication
        .tick_label
        .as_deref()
        .and_then(|label| label.strip_prefix("Tick "))
        .and_then(|label| label.split_once(' '))
        .context("published tick label absent")?
        .0
        .parse()
        .context("published tick label malformed")
}

fn clock_minute(publication: &Published) -> Result<i64> {
    let clock = publication
        .clock_fallback
        .as_deref()
        .context("published household clock absent")?;
    let (day, time) = clock
        .strip_prefix("Day ")
        .and_then(|clock| clock.split_once(" · "))
        .context("published household clock malformed")?;
    let (hour, minute) = time
        .split_once(':')
        .context("published clock time malformed")?;
    let day = day.parse::<i64>()?;
    let hour = hour.parse::<i64>()?;
    let minute = minute.parse::<i64>()?;
    ensure!(
        day > 0 && (0..24).contains(&hour) && (0..60).contains(&minute),
        "published clock out of range"
    );
    Ok((day - 1) * 1440 + hour * 60 + minute)
}

fn serials(publication: &Published) -> Vec<u64> {
    publication
        .queue
        .iter()
        .map(|entry| u64::from(entry.action.serial))
        .collect()
}

fn managed_difference_paths(before: &Value, after: &Value) -> Vec<String> {
    fn collect(before: &Value, after: &Value, path: &str, differences: &mut Vec<String>) {
        if differences.len() >= 32 {
            return;
        }
        match (before, after) {
            (Value::Object(before), Value::Object(after)) => {
                let keys: BTreeSet<_> = before.keys().chain(after.keys()).collect();
                for key in keys {
                    let next = format!("{path}/{}", key.replace('~', "~0").replace('/', "~1"));
                    match (before.get(key), after.get(key)) {
                        (Some(before), Some(after)) => collect(before, after, &next, differences),
                        _ => differences.push(next),
                    }
                    if differences.len() >= 32 {
                        break;
                    }
                }
            }
            (Value::Array(before), Value::Array(after)) if before.len() == after.len() => {
                for (index, (before, after)) in before.iter().zip(after).enumerate() {
                    collect(before, after, &format!("{path}/{index}"), differences);
                    if differences.len() >= 32 {
                        break;
                    }
                }
            }
            (Value::Number(before), Value::Number(after)) => {
                if before != after
                    || before.as_f64().map(f64::to_bits) != after.as_f64().map(f64::to_bits)
                {
                    differences.push(path.to_owned());
                }
            }
            _ if before != after => differences.push(path.to_owned()),
            _ => {}
        }
    }
    let mut differences = Vec::new();
    collect(before, after, "", &mut differences);
    differences
}

fn accounted_camera_republication(before: &Value, after: &Value) -> Result<Value> {
    let differences = managed_difference_paths(before, after);
    ensure!(
        differences
            == vec![
                "/camera_device/generation".to_owned(),
                "/camera_device/published_generation".to_owned(),
            ],
        "managed state has unaccounted differences: {differences:?}"
    );
    let generation = |value: &Value, field: &str| -> Result<u64> {
        value
            .pointer(&format!("/camera_device/{field}"))
            .and_then(Value::as_u64)
            .context("managed camera generation absent")
    };
    let prepared_before = generation(before, "generation")?;
    let prepared_after = generation(after, "generation")?;
    let published_before = generation(before, "published_generation")?;
    let published_after = generation(after, "published_generation")?;
    ensure!(
        prepared_before == published_before
            && prepared_after == published_after
            && prepared_before.checked_add(2) == Some(prepared_after),
        "managed camera republication is not the expected two completed refreshes"
    );
    Ok(
        json!({"difference_paths":differences,"prepared_before":prepared_before,
        "prepared_after":prepared_after,"published_before":published_before,
        "published_after":published_after,"all_other_fields_equal":true,
        "normalizations":[],"expected_refresh_count":2}),
    )
}

#[test]
fn camera_republication_proof_is_exhaustive_without_snapshot_changes() -> Result<()> {
    let before = json!({"session":{"tick":3,"rng":7,"zero":-0.0},"camera_device":{
        "generation":8,"published_generation":8},"resident":16});
    let after = json!({"session":{"tick":3,"rng":7,"zero":-0.0},"camera_device":{
        "generation":10,"published_generation":10},"resident":16});
    let original_before = before.clone();
    let original_after = after.clone();
    let proof = accounted_camera_republication(&before, &after)?;
    ensure!(proof["all_other_fields_equal"] == true);
    ensure!(before == original_before && after == original_after);
    ensure!(before != after);
    Ok(())
}

#[test]
fn camera_republication_proof_rejects_unrelated_and_incomplete_changes() {
    let before = json!({"session":{"tick":3,"rng":7,"zero":-0.0},"camera_device":{
        "generation":8,"published_generation":8},"resident":16});
    let valid = json!({"session":{"tick":3,"rng":7,"zero":-0.0},"camera_device":{
        "generation":10,"published_generation":10},"resident":16});
    let mut changed_session = valid.clone();
    changed_session["session"]["tick"] = json!(4);
    let mut missing_field = valid.clone();
    missing_field.as_object_mut().unwrap().remove("resident");
    let mut added_null = valid.clone();
    added_null["extra"] = Value::Null;
    let mut incomplete = valid.clone();
    incomplete["camera_device"]["published_generation"] = json!(9);
    let mut wrong_delta = valid.clone();
    wrong_delta["camera_device"]["generation"] = json!(12);
    wrong_delta["camera_device"]["published_generation"] = json!(12);
    let mut changed_float_bits = valid.clone();
    changed_float_bits["session"]["zero"] = json!(0.0);
    for after in [
        changed_session,
        missing_field,
        added_null,
        incomplete,
        wrong_delta,
        changed_float_bits,
    ] {
        assert!(accounted_camera_republication(&before, &after).is_err());
    }
}

fn observation(publication: &Published) -> Value {
    json!({"tick_label":publication.tick_label,"clock":publication.clock_fallback,
        "session_present":publication.session_present,"playing":publication.playing,
        "pending":publication.pending,"modal":publication.modal,"error":publication.error,
        "status":publication.status,"resident":publication.resident,
        "native_selection":publication.native_selection,
        "residents":publication.residents.iter().map(|resident| json!({"object":resident.object,
            "neighbor":resident.neighbor,"name":resident.name,"enabled":resident.enabled,
            "highlighted":resident.highlighted})).collect::<Vec<_>>(),
        "controls":publication.controls.iter().map(|(kind, button)| json!({
            "kind":format!("{kind:?}"),"enabled":button.enabled,"selected":button.selected,
            "auxiliary":button.auxiliary})).collect::<Vec<_>>(),
        "active_panel":publication.active_panel,"panel_state":publication.panel_state,
        "root_visible":publication.root_visible,"people_visible":publication.people_visible,
        "menu":publication.menu.as_ref().map(|menu| json!({"title":menu.title,
            "ready":menu.ready,"preview":menu.preview,"visible":menu.popup_visible,
            "error":menu.error,"offered":menu.popup.iter().map(|choice| &choice.label).collect::<Vec<_>>()})),
        "queue":publication.queue.iter().map(|entry| json!({"label":entry.label,
            "diagnostic_label":entry.diagnostic_label,"target":entry.target,
            "serial":entry.action.serial,"active":entry.action.active,
            "cancellation_requested":entry.action.cancel_requested})).collect::<Vec<_>>(),
        "native_popup":publication.native_popup,"native_popup_epoch":publication.native_popup_epoch,
        "dialog":publication.dialog,"configuration":publication.application})
}

fn require_native_resident(publication: &Published, resident: i16) -> Result<()> {
    let selected = publication
        .native_selection
        .as_ref()
        .context("native selection observation has no Session")?
        .as_ref()
        .map_err(|error| anyhow::anyhow!("native selection getter failed: {error}"))?;
    ensure!(
        *selected == Some(resident) && publication.resident == resident,
        "native resident ownership and published selection border disagree: native={selected:?}, border={}, requested={resident}",
        publication.resident
    );
    Ok(())
}

fn authoritative_mode(publication: &Published) -> Option<i32> {
    publication.cluster.as_ref()?.values.mode_raw
}

fn live_disclosure_available(mode: Option<i32>, button: Option<ButtonView>) -> bool {
    mode == Some(2) && button.is_some_and(|button| button.enabled)
}

fn expanded_people_panel(state: Option<i32>, root_visible: bool, people_visible: bool) -> bool {
    state == Some(0) && root_visible && people_visible
}

fn resident_availability(residents: &[Resident], panel_visible: bool) -> Vec<Value> {
    residents
        .iter()
        .map(|resident| {
            let reason = if !resident.enabled {
                Some("no enabled current live-resident/interaction-actor control; historical household membership is not availability")
            } else if !panel_visible {
                Some("People controls are not visible in a raw-expanded panel")
            } else {
                None
            };
            json!({"object":resident.object,"neighbor":resident.neighbor,"name":resident.name,
                "enabled":resident.enabled,"available":reason.is_none(),"unavailable_reason":reason})
        })
        .collect()
}

fn resident_selection_coverage(residents: &[Resident], native_selected: &[i16]) -> Value {
    let available: BTreeSet<_> = residents
        .iter()
        .filter(|resident| resident.enabled)
        .map(|resident| resident.object)
        .collect();
    let selected: BTreeSet<_> = native_selected
        .iter()
        .copied()
        .filter(|resident| available.contains(resident))
        .collect();
    let all_selected = !residents.is_empty()
        && residents
            .iter()
            .all(|resident| resident.enabled && selected.contains(&resident.object));
    let switched = selected.len() >= 2;
    let qualified = all_selected && switched;
    json!({"status":if qualified {"passed"} else if selected.is_empty() {"unsupported"} else {"partial"},
        "selection_scope":"available-native-selection",
        "available_resident_count":available.len(),"native_selected_resident_count":selected.len(),
        "household_roster_selection_qualified":all_selected,
        "multi_resident_switching":{"status":if switched {"passed"} else {"unsupported"},
            "qualified":switched,"distinct_native_selected_residents":selected,
            "reason":(!switched).then_some("fewer than two enabled current residents were natively selected; multi-resident switching is NOT qualified")},
        "reason":(!qualified).then_some("available native selections do not qualify full household selection and multi-resident switching; no tutorial completion claim")})
}

fn post_live_resident(residents: &[Resident], previous: i16) -> Option<i16> {
    residents
        .iter()
        .find(|resident| resident.enabled && resident.object != previous)
        .or_else(|| residents.iter().find(|resident| resident.enabled))
        .map(|resident| resident.object)
}

#[cfg(test)]
fn synthetic_residents(entries: &[(i16, bool)]) -> Vec<Resident> {
    entries
        .iter()
        .map(|&(object, enabled)| Resident {
            object,
            neighbor: object,
            name: format!("Synthetic {object}"),
            enabled,
            highlighted: false,
        })
        .collect()
}

#[test]
fn live_disclosure_requires_authoritative_mode_and_enabled_control() {
    let live = ButtonView {
        enabled: true,
        selected: false,
        auxiliary: 0,
    };
    assert!(live_disclosure_available(Some(2), Some(live)));
    assert!(!live_disclosure_available(None, Some(live)));
    assert!(!live_disclosure_available(Some(1), Some(live)));
    assert!(!live_disclosure_available(Some(2), None));
    assert!(!live_disclosure_available(
        Some(2),
        Some(ButtonView {
            enabled: false,
            selected: true,
            ..live
        }),
    ));
    assert!(!live_disclosure_available(
        Some(1),
        Some(ButtonView {
            selected: true,
            ..live
        }),
    ));
}

#[test]
fn live_disclosure_requires_raw_expanded_visible_people_panel() {
    assert!(expanded_people_panel(Some(0), true, true));
    for state in [None, Some(1), Some(2), Some(-1)] {
        assert!(!expanded_people_panel(state, true, true));
    }
    for (root, people) in [(false, false), (false, true), (true, false)] {
        assert!(!expanded_people_panel(Some(0), root, people));
    }
}

#[test]
fn resident_availability_preserves_unavailable_household_members() {
    let residents = synthetic_residents(&[(105, true), (137, false)]);
    let roster = resident_availability(&residents, true);
    assert_eq!(roster.len(), 2);
    assert_eq!(roster[0]["object"], 105);
    assert_eq!(roster[0]["available"], true);
    assert!(roster[0]["unavailable_reason"].is_null());
    assert_eq!(roster[1]["object"], 137);
    assert_eq!(roster[1]["neighbor"], 137);
    assert_eq!(roster[1]["name"], "Synthetic 137");
    assert_eq!(roster[1]["available"], false);
    assert!(roster[1]["unavailable_reason"].is_string());
    let hidden = resident_availability(&residents, false);
    assert_eq!(hidden.len(), 2);
    assert_eq!(hidden[0]["enabled"], true);
    assert_eq!(hidden[0]["available"], false);
    assert!(hidden[0]["unavailable_reason"].is_string());
}

#[test]
fn resident_selection_single_available_is_partial_without_switching_credit() {
    for entries in [vec![(105, true), (137, false)], vec![(105, true)]] {
        let residents = synthetic_residents(&entries);
        for selected in [vec![105], vec![105, 105], vec![105, 137], vec![105, 999]] {
            let receipt = resident_selection_coverage(&residents, &selected);
            assert_eq!(receipt["status"], "partial");
            assert_eq!(receipt["selection_scope"], "available-native-selection");
            assert_eq!(receipt["available_resident_count"], 1);
            assert_eq!(receipt["native_selected_resident_count"], 1);
            assert_eq!(receipt["multi_resident_switching"]["qualified"], false);
            assert_eq!(receipt["multi_resident_switching"]["status"], "unsupported");
            assert!(receipt["multi_resident_switching"]["reason"].is_string());
        }
    }
}

#[test]
fn resident_selection_requires_full_roster_and_distinct_native_selections() {
    let residents = synthetic_residents(&[(105, true), (137, true)]);
    let complete = resident_selection_coverage(&residents, &[105, 137]);
    assert_eq!(complete["status"], "passed");
    assert_eq!(complete["household_roster_selection_qualified"], true);
    assert_eq!(complete["multi_resident_switching"]["qualified"], true);
    let incomplete = resident_selection_coverage(&residents, &[105, 105]);
    assert_eq!(incomplete["status"], "partial");
    assert_eq!(incomplete["household_roster_selection_qualified"], false);
    assert_eq!(incomplete["multi_resident_switching"]["qualified"], false);
    let residents = synthetic_residents(&[(105, true), (137, true), (200, false)]);
    let partial = resident_selection_coverage(&residents, &[105, 137]);
    assert_eq!(partial["status"], "partial");
    assert_eq!(partial["household_roster_selection_qualified"], false);
    assert_eq!(partial["multi_resident_switching"]["qualified"], true);
    for residents in [vec![], synthetic_residents(&[(105, false), (137, false)])] {
        let unavailable = resident_selection_coverage(&residents, &[105, 137]);
        assert_eq!(unavailable["status"], "unsupported");
        assert_eq!(unavailable["multi_resident_switching"]["qualified"], false);
    }
}

#[test]
fn buy_live_never_invents_a_distinct_available_resident() {
    let single = synthetic_residents(&[(105, true), (137, false)]);
    assert_eq!(post_live_resident(&single, 105), Some(105));
    assert_eq!(post_live_resident(&single, 137), Some(105));
    let multiple = synthetic_residents(&[(105, true), (137, true)]);
    assert_eq!(post_live_resident(&multiple, 105), Some(137));
    assert_eq!(post_live_resident(&multiple, 137), Some(105));
    assert_eq!(post_live_resident(&[], 105), None);
    assert_eq!(
        post_live_resident(&synthetic_residents(&[(137, false)]), 105),
        None,
    );
    let coverage = resident_selection_coverage(&single, &[105, 105]);
    assert_eq!(coverage["multi_resident_switching"]["qualified"], false);
}

#[test]
fn resident_partial_or_unsupported_never_qualifies_all_required_operations() {
    for residents in [
        synthetic_residents(&[(105, true), (137, false)]),
        synthetic_residents(&[(105, true)]),
        synthetic_residents(&[(105, false), (137, false)]),
        vec![],
    ] {
        let receipt = resident_selection_coverage(&residents, &[105]);
        let operations = json!({"initial_play":{"status":"passed"},
            "resident_selection":receipt,"buy_live":{"status":"passed"},
            "interaction_queue_cancel":{"status":"passed"},"managed_save_load":{"status":"passed"},
            "post_reload_play":{"status":"passed"}});
        assert!(operations["resident_selection"]["status"] != "passed");
        assert!(
            !operations
                .as_object()
                .unwrap()
                .values()
                .all(|operation| operation["status"] == "passed")
        );
    }
}

impl Journey {
    fn timing_observation(&self) -> player_timing::Observation {
        let publication = self.host.publication_for_test();
        player_timing::Observation {
            generation: self.host.current_generation_for_test(),
            session_present: publication.session_present,
            pending: publication.pending,
            modal: publication.modal,
            playing: publication.playing,
            application_error: publication.error.is_some(),
            menu_error: publication
                .menu
                .as_ref()
                .is_some_and(|menu| menu.error.is_some()),
            owner_drained: self.idle && self.streams == 0 && self.host.frontend_idle_for_test(),
            tick: tick(publication).ok(),
            clock_minute: clock_minute(publication).ok(),
            commands: self.commands,
            frames: self.frames,
            service_polls: self.service_polls,
            effect_polls: self.effect_polls,
            task_streams: self.tasks,
        }
    }

    fn timing_checkpoint(&self) {
        if let Some(timing) = &self.timing {
            timing.observe_settled(self.timing_observation());
        }
    }

    fn timing_success(&self) {
        if let Some(timing) = &self.timing {
            let end = timing.timestamp();
            timing.finish_checkpoint(self.timing_parent, end, self.timing_observation());
        }
    }

    fn timed<Reply: TimingOutcome>(
        &mut self,
        scope: Scope,
        operation: Operation,
        role: Role,
        call: impl FnOnce(&mut Self) -> Result<Reply>,
    ) -> Result<Reply> {
        let Some(timing) = self.timing.clone() else {
            return call(self);
        };
        let parent = self.timing_parent;
        let id = timing.start(
            parent,
            scope,
            operation,
            role,
            Some(self.timing_observation()),
            scope == Scope::NativeProof,
        );
        self.timing_parent = id;
        let result = call(self);
        let end = timing.timestamp();
        self.timing_parent = parent;
        let error = result.as_ref().err().map(|error| format!("{error:#}"));
        timing.finish_at(
            id,
            end,
            result
                .as_ref()
                .map_or(Outcome::Error, TimingOutcome::timing_outcome),
            Some(self.timing_observation()),
            error.as_deref(),
        );
        result
    }

    fn bounded(&self) -> Result<()> {
        ensure!(
            self.wall.elapsed().as_secs_f64() < self.wall_seconds,
            "Host journey wall limit"
        );
        ensure!(self.commands < MAX_COMMANDS, "Host journey command limit");
        ensure!(self.frames < MAX_FRAMES, "Host journey frame limit");
        ensure!(
            self.service_polls < MAX_POLLS && self.effect_polls < MAX_POLLS,
            "Host journey poll limit"
        );
        ensure!(self.tasks < MAX_COMMANDS, "Host journey task-stream limit");
        Ok(())
    }

    fn healthy(&self) -> Result<()> {
        let publication = self.host.publication_for_test();
        ensure!(
            publication.error.is_none(),
            "Host application error: {:?}",
            publication.error
        );
        ensure!(
            publication
                .menu
                .as_ref()
                .is_none_or(|menu| menu.error.is_none()),
            "Host menu error"
        );
        ensure!(
            !publication.modal || publication.dialog.is_some(),
            "Host modal record absent"
        );
        ensure!(
            publication.audit.is_none(),
            "per-command full audit must remain disabled"
        );
        Ok(())
    }

    fn submit(&mut self, task: Task<Event>) -> Result<()> {
        if let Some(mut stream) = iced_runtime::task::into_stream(task) {
            self.bounded()?;
            self.streams += 1;
            self.tasks += 1;
            self.idle = false;
            let sender = self.sender.clone();
            self.executor.spawn_ok(async move {
                while let Some(action) = stream.next().await {
                    if sender.send(Delivery::Action(action)).is_err() {
                        return;
                    }
                }
                let _ = sender.send(Delivery::Finished);
            });
        }
        Ok(())
    }

    fn drive(&mut self, task: Task<Event>) -> Result<()> {
        self.idle = false;
        self.submit(task)?;
        loop {
            self.bounded()?;
            if self.streams == 0 {
                for effect in self.host.settle_for_test()? {
                    self.submit(effect)?;
                }
                if self.streams == 0 {
                    self.idle = true;
                    return self.healthy();
                }
            }
            self.effect_polls += 1;
            let remaining = (self.wall_seconds - self.wall.elapsed().as_secs_f64()).max(0.001);
            let delivery = match self
                .receiver
                .recv_timeout(Duration::from_secs_f64(remaining.min(0.1)))
            {
                Ok(delivery) => delivery,
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(error) => return Err(error.into()),
            };
            match delivery {
                Delivery::Finished => self.streams -= 1,
                Delivery::Action(iced_runtime::Action::Output(event)) => {
                    self.commands += 1;
                    if let Event::Effect {
                        generation,
                        message: Message::Loaded(result),
                    } = &event
                    {
                        if let Some(timing) = &self.timing {
                            timing.loaded_effect(
                                *generation,
                                self.host.current_generation_for_test(),
                            );
                        }
                        self.events.push(
                            json!({"kind":"actual_loaded_effect","generation":generation,
                            "owner_generation_before":self.host.current_generation_for_test(),
                            "load_error":result.as_ref().err()}),
                        );
                    } else if !matches!(event, Event::Completion) {
                        self.events
                            .push(json!({"kind":"task_output","event":format!("{event:?}")}));
                    }
                    let next = self.host.update(event);
                    self.submit(next)?;
                }
                Delivery::Action(action) => {
                    self.events.push(json!({"kind":"unsupported_runtime_action",
                        "action":format!("{action:?}"),"completed":false}));
                    anyhow::bail!(
                        "native runtime action requires a real runtime; not dropped as completed"
                    );
                }
            }
        }
    }

    fn ui(&mut self, message: Message) -> Result<()> {
        self.ui_with_role(message, Role::PlayerPath)
    }

    fn ui_with_role(&mut self, message: Message, role: Role) -> Result<()> {
        self.bounded()?;
        self.healthy()?;
        self.commands += 1;
        self.events.push(json!({"kind":"ui_input","generation":self.host.current_generation_for_test(),
            "message":format!("{message:?}"),"tick_label":self.host.publication_for_test().tick_label}));
        let operation = self.timing.as_ref().map(|_| match &message {
            Message::Save => Operation::ProductionSave,
            Message::Load => Operation::ManagedLoad,
            Message::Resident(_) => Operation::ResidentSelection,
            Message::Buy => Operation::Buy,
            Message::Live => Operation::Live,
            Message::ObjectChoice(_) | Message::GroundChoice(_) => Operation::OfferedInteraction,
            Message::Cancel(_) => Operation::Cancellation,
            _ => Operation::OtherUi,
        });
        let event = self
            .host
            .ui_event_for_test(self.host.publication_for_test(), message);
        let Some(timing) = self.timing.clone() else {
            let task = self.host.update(event);
            return self.drive(task);
        };
        let before = self.timing_observation();
        let generation = before.generation;
        let operation = operation.unwrap();
        let id = timing.start(
            self.timing_parent,
            Scope::UiDispatch,
            operation,
            role,
            Some(before),
            true,
        );
        timing.expect_generation(
            id,
            generation,
            if operation == Operation::ManagedLoad {
                generation.checked_add(1)
            } else {
                Some(generation)
            },
        );
        let task = timing.dispatch(id, || self.host.update(event));
        let result = self.drive(task);
        let returned = timing.timestamp();
        let after = self.timing_observation();
        timing.drive_return_at(id, returned, after.owner_drained);
        let error = result.as_ref().err().map(|error| format!("{error:#}"));
        timing.finish_at(
            id,
            returned,
            if result.is_ok() {
                Outcome::Completed
            } else {
                Outcome::Error
            },
            Some(after),
            error.as_deref(),
        );
        result
    }

    fn frame(&mut self, advance: bool) -> Result<()> {
        self.bounded()?;
        if advance {
            self.frames += 1;
            self.millis += 40;
        } else {
            self.service_polls += 1;
        }
        self.commands += 1;
        let task = self.host.update(Event::Frame(
            self.epoch + Duration::from_millis(self.millis),
        ));
        self.drive(task)?;
        if advance && (self.frames == 1 || self.frames % 256 == 0) {
            self.events
                .push(json!({"kind":"frame_sample","frames":self.frames,
                "tick_label":self.host.publication_for_test().tick_label,
                "clock":self.host.publication_for_test().clock_fallback,
                "generation":self.host.current_generation_for_test(),
                "wall_seconds":self.wall.elapsed().as_secs_f64()}));
        }
        Ok(())
    }

    fn settle_presentation(&mut self) -> Result<()> {
        for _ in 0..10_000 {
            self.healthy()?;
            ensure!(
                !self.host.publication_for_test().modal,
                "unexpected modal during player operation"
            );
            if !self.host.publication_for_test().pending {
                self.timing_checkpoint();
                return Ok(());
            }
            self.frame(false)?;
        }
        anyhow::bail!("Host presentation service-poll limit")
    }

    fn pause(&mut self) -> Result<()> {
        self.settle_presentation()?;
        if self.host.publication_for_test().playing {
            self.ui(Message::Toggle)?;
            self.settle_presentation()?;
        }
        ensure!(
            !self.host.publication_for_test().playing,
            "Host did not pause"
        );
        Ok(())
    }

    fn play(&mut self, phase: &str, minutes: Option<i64>, cancelled: Option<u64>) -> Result<Value> {
        let operation = match phase {
            "initial_play" => Operation::InitialPlay,
            "post_reload_play" => Operation::PostReloadPlay,
            _ => Operation::CancelContinuation,
        };
        self.timed(Scope::Play, operation, Role::PlayerPath, |journey| {
            journey.play_inner(phase, minutes, cancelled)
        })
    }

    fn play_inner(
        &mut self,
        phase: &str,
        minutes: Option<i64>,
        cancelled: Option<u64>,
    ) -> Result<Value> {
        let start_tick = tick(self.host.publication_for_test())?;
        let start_minute = clock_minute(self.host.publication_for_test())?;
        let mut requested = false;
        let mut answered = false;
        loop {
            self.bounded()?;
            self.healthy()?;
            let publication = self.host.publication_for_test().clone();
            let now_tick = tick(&publication)?;
            let now_minute = clock_minute(&publication)?;
            ensure!(
                now_tick >= start_tick && now_tick - start_tick < MAX_TICKS,
                "Host phase tick limit/backwards tick"
            );
            ensure!(now_minute >= start_minute, "Host calendar moved backwards");
            if !publication.pending
                && !publication.modal
                && publication.playing
                && now_tick > start_tick
                && minutes.is_none_or(|minutes| {
                    now_tick - start_tick >= 32 && now_minute - start_minute >= minutes
                })
                && cancelled.is_none_or(|serial| !serials(&publication).contains(&serial))
            {
                self.timing_success();
                self.timing_checkpoint();
                return Ok(
                    json!({"status":"passed","phase":phase,"start_tick":start_tick,
                    "tick":now_tick,"start_clock_minute":start_minute,"clock_minute":now_minute,
                    "observed_clock_minute_boundaries":now_minute-start_minute,
                    "cancelled_serial_absent":cancelled,"generation":self.host.current_generation_for_test()}),
                );
            }
            let input = next_input(
                publication.error.as_deref(),
                publication
                    .modal
                    .then(|| publication.dialog.as_ref().map(|record| record.id))
                    .flatten(),
                publication.pending,
                publication.playing,
                requested,
                answered,
            )?;
            match input {
                NextInput::Poll => self.frame(false)?,
                NextInput::Frame => self.frame(true)?,
                NextInput::Play => {
                    self.ui(Message::Toggle)?;
                    requested = true;
                    answered = false;
                }
                NextInput::Dialog(id) => {
                    let record = publication
                        .dialog
                        .as_ref()
                        .context("Host startup modal absent")?;
                    self.events.push(json!({"kind":"modal","phase":phase,"record":record,
                        "body_sha256":engine::digest(record.body.as_bytes()),"response_status":"not_sent"}));
                    ensure!(
                        phase == "initial_play",
                        "unexpected dialog outside fresh startup; no automatic answer"
                    );
                    ensure!(self.dialogs < 16, "Host startup dialog limit");
                    let response = startup_response(record)?;
                    ensure!(
                        record.status == 1 && record.modal(),
                        "startup dialog not answerable"
                    );
                    let choice = record
                        .buttons
                        .iter()
                        .find(|button| button.result == response)
                        .context("pinned Host dialog button absent")?
                        .label
                        .clone();
                    let receipt = self.events.len() - 1;
                    self.events[receipt]["response"] = json!(response);
                    self.events[receipt]["choice"] = json!(choice);
                    self.events[receipt]["response_status"] = json!("attempted");
                    self.ui(Message::Dialog(id, response))?;
                    self.events[receipt]["response_status"] = json!("task_streams_drained");
                    self.dialogs += 1;
                    answered = true;
                }
            }
        }
    }

    fn residents(&mut self) -> Result<Value> {
        self.pause()?;
        let before = observation(self.host.publication_for_test());
        let disclosure = self.expand_live_panel()?;
        let residents = self.host.publication_for_test().residents.clone();
        let roster = resident_availability(&residents, true);
        let mut selected = Vec::new();
        let mut native_selected = Vec::new();
        for resident in residents.iter().filter(|resident| resident.enabled) {
            let publication = self.host.publication_for_test();
            ensure!(
                expanded_people_panel(
                    publication.panel_state,
                    publication.root_visible,
                    publication.people_visible,
                ) && live_disclosure_available(
                    authoritative_mode(publication),
                    publication.button(Widget::Live),
                ) && publication
                    .residents
                    .iter()
                    .any(|entry| entry.object == resident.object && entry.enabled),
                "resident {} has no visible enabled current control",
                resident.object
            );
            let selected_resident = self.timed(Scope::NativeProof, Operation::ResidentSelection, Role::PlayerPath, |journey| {
                journey.ui(Message::Resident(resident.object))?;
                journey.settle_presentation()?;
                let publication = journey.host.publication_for_test();
                ensure!(publication.resident == resident.object, "published resident selection differs");
                require_native_resident(publication, resident.object)?;
                journey.timing_success();
                Ok(json!({"object":resident.object,"name":resident.name,
                    "native_selection":publication.native_selection,
                    "tutorial_highlighted":publication.residents.iter().find(|entry| entry.object == resident.object).map(|entry| entry.highlighted),
                    "evidence_scope":"production Resident handler, exact native selected-person getter and selection-border input; tutorial highlight is unrelated"}))
            })?;
            native_selected.push(resident.object);
            selected.push(selected_resident);
        }
        let mut receipt = resident_selection_coverage(&residents, &native_selected);
        receipt["before_disclosure"] = before;
        receipt["live_disclosure"] = disclosure;
        receipt["household_roster"] = json!(roster);
        receipt["selected"] = json!(selected);
        Ok(receipt)
    }

    fn expand_live_panel(&mut self) -> Result<Value> {
        self.pause()?;
        let publication = self.host.publication_for_test();
        ensure!(
            live_disclosure_available(
                authoritative_mode(publication),
                publication.button(Widget::Live),
            ),
            "authoritative Live mode or real enabled Live control unavailable for panel disclosure"
        );
        ensure!(
            matches!(publication.panel_state, Some(0) | Some(1)),
            "raw panel expansion state unavailable for real Live disclosure"
        );
        let before = json!({"people_visible":publication.people_visible,
            "root_visible":publication.root_visible,"panel_state":publication.panel_state,
            "mode_raw":authoritative_mode(publication),
            "live_selected":publication.button(Widget::Live).map(|button| button.selected)});
        let requested = publication.panel_state == Some(1);
        if requested {
            self.ui(Message::Live)?;
            self.settle_presentation()?;
        }
        let publication = self.host.publication_for_test();
        ensure!(
            expanded_people_panel(
                publication.panel_state,
                publication.root_visible,
                publication.people_visible,
            ) && live_disclosure_available(
                authoritative_mode(publication),
                publication.button(Widget::Live),
            ),
            "real Live disclosure did not expose raw-expanded People controls in authoritative Live mode"
        );
        Ok(
            json!({"status":"passed","live_pressed_to_expand":requested,"before":before,
            "after":{"people_visible":publication.people_visible,
                "root_visible":publication.root_visible,"panel_state":publication.panel_state,
                "mode_raw":authoritative_mode(publication),
                "live_enabled":publication.button(Widget::Live).map(|button| button.enabled),
                "live_selected":publication.button(Widget::Live).map(|button| button.selected)}}),
        )
    }

    fn buy_live(&mut self) -> Result<Value> {
        self.pause()?;
        let publication = self.host.publication_for_test();
        if !publication.can_buy
            || !publication
                .button(Widget::Buy)
                .is_some_and(|button| button.enabled)
        {
            return Ok(
                json!({"status":"unsupported","reason":"real Buy owner/control unavailable",
                "observation":observation(publication)}),
            );
        }
        let buy = self.timed(
            Scope::NativeProof,
            Operation::Buy,
            Role::PlayerPath,
            |journey| {
                journey.ui(Message::Buy)?;
                journey.settle_presentation()?;
                ensure!(
                    journey
                        .host
                        .publication_for_test()
                        .button(Widget::Buy)
                        .is_some_and(|button| button.selected),
                    "published Buy selection absent"
                );
                journey.timing_success();
                Ok(observation(journey.host.publication_for_test()))
            },
        )?;
        ensure!(
            self.host
                .publication_for_test()
                .button(Widget::Live)
                .is_some_and(|button| button.enabled),
            "real Live control unavailable after Buy"
        );
        self.timed(
            Scope::NativeProof,
            Operation::Live,
            Role::PlayerPath,
            |journey| {
                journey.ui(Message::Live)?;
                journey.settle_presentation()?;
                ensure!(
                    authoritative_mode(journey.host.publication_for_test()) == Some(2),
                    "authoritative Live mode absent after real Live dispatch"
                );
                journey.timing_success();
                Ok(())
            },
        )?;
        let disclosure = self.expand_live_panel()?;
        let publication = self.host.publication_for_test();
        let previous_resident = publication.resident;
        let residents = publication.residents.clone();
        let mut receipt = json!({"status":"unsupported","buy":buy,"live":observation(publication),
            "buy_live_transition":{"status":"passed","live_mode_raw":authoritative_mode(publication)},
            "live_disclosure":disclosure,"household_roster":resident_availability(&residents, true),
            "previous_resident":previous_resident,"resident_reselected_after_live":null,
            "native_resident_changed":false,
            "multi_resident_switching":resident_selection_coverage(&residents, &[])["multi_resident_switching"],
            "reason":"no enabled current resident control after Live; transition alone does not qualify resident dispatch or switching",
            "evidence_scope":"real handlers, visible controls and native selection; not a full owner audit or tutorial completion"});
        let Some(resident) = post_live_resident(&residents, previous_resident) else {
            return Ok(receipt);
        };
        require_native_resident(publication, previous_resident)?;
        self.ui(Message::Resident(resident))?;
        self.settle_presentation()?;
        require_native_resident(self.host.publication_for_test(), resident)?;
        let coverage = resident_selection_coverage(&residents, &[previous_resident, resident]);
        let switched = coverage["multi_resident_switching"]["qualified"] == true;
        receipt["status"] = json!(if switched { "passed" } else { "partial" });
        receipt["live"] = observation(self.host.publication_for_test());
        receipt["resident_reselected_after_live"] = json!(resident);
        receipt["native_resident_changed"] = json!(resident != previous_resident);
        receipt["multi_resident_switching"] = coverage["multi_resident_switching"].clone();
        receipt["reason"] = json!((!switched).then_some(
            "only available native selection was re-dispatched; distinct-resident switching is NOT qualified",
        ));
        Ok(receipt)
    }

    fn visible_targets(&self) -> Result<VisibleTargets> {
        let publication = self.host.publication_for_test();
        let scene = publication
            .scene
            .as_ref()
            .context("published scene absent")?;
        let mut object_indices = BTreeMap::new();
        let mut candidates: Vec<(i16, Vec<[f32; 2]>)> = Vec::new();
        let mut priorities = Vec::new();
        let mut excluded = TargetExclusions::default();
        for draw in &scene.draws {
            let Some(object) = draw.object else { continue };
            if object <= 0 {
                continue;
            }
            let Some(priority) =
                excluded.priority(object, publication.journey_targets.get(&object))
            else {
                continue;
            };
            for triangle in scene.vertices[draw.vertices.start as usize..draw.vertices.end as usize]
                .chunks_exact(3)
            {
                self.bounded()?;
                let pixel: [f32; 2] = [
                    triangle
                        .iter()
                        .map(|vertex| (vertex.position[0] + 1.) * scene.width as f32 / 6.)
                        .sum(),
                    triangle
                        .iter()
                        .map(|vertex| (1. - vertex.position[1]) * scene.height as f32 / 6.)
                        .sum(),
                ];
                if pixel.iter().all(|coordinate| coordinate.is_finite())
                    && pixel[0] >= 0.
                    && pixel[1] >= 0.
                    && pixel[0] < scene.width as f32
                    && pixel[1] < scene.height as f32
                {
                    let candidate_index = *object_indices.entry(object).or_insert_with(|| {
                        let candidate_index = candidates.len();
                        candidates.push((object, Vec::new()));
                        priorities.push(priority);
                        candidate_index
                    });
                    candidates[candidate_index].1.push(pixel);
                }
            }
        }
        let discovery_limit = MAX_HIT_TESTS - MAX_TARGETS;
        let object_indices = prioritized_indices(&priorities, discovery_limit);
        let probed_objects = object_indices.len();
        let mut discovered = Vec::new();
        let mut discovered_priorities = Vec::new();
        let mut probes = 0;
        for (position, candidate_index) in object_indices.into_iter().enumerate() {
            let (object, centroids) = &candidates[candidate_index];
            let allowance = discovery_probe_allowance(position, probed_objects);
            for centroid_index in spread_indices(centroids.len(), allowance) {
                self.bounded()?;
                let pixel = centroids[centroid_index];
                probes += 1;
                if scene.pick(pixel) == Some(*object) {
                    discovered.push((*object, pixel));
                    discovered_priorities.push(priorities[candidate_index]);
                    break;
                }
            }
        }
        let sampled_indices = prioritized_indices(&discovered_priorities, MAX_TARGETS);
        let sit_sampled = sampled_indices
            .iter()
            .filter(|&&index| discovered_priorities[index])
            .count();
        let sampled = sampled_indices
            .into_iter()
            .map(|index| discovered[index])
            .collect();
        Ok(VisibleTargets {
            candidate_objects: candidates.len(),
            candidate_centroids: candidates
                .iter()
                .map(|(_, centroids)| centroids.len())
                .sum(),
            probed_objects,
            discovered,
            sampled,
            probes,
            excluded,
            sit_candidates: priorities.iter().filter(|&&priority| priority).count(),
            sit_discovered: discovered_priorities
                .iter()
                .filter(|&&priority| priority)
                .count(),
            sit_sampled,
        })
    }

    fn interaction(&mut self, receipt: &mut Value) -> Result<()> {
        self.pause()?;
        let disclosure = self.expand_live_panel()?;
        let resident = self.host.publication_for_test().resident;
        ensure!(
            self.host
                .publication_for_test()
                .residents
                .iter()
                .any(|entry| entry.object == resident && entry.enabled),
            "real interaction resident unavailable"
        );
        self.ui(Message::Resident(resident))?;
        self.settle_presentation()?;
        *receipt = json!({"status":"checking_selection","resident":resident,"live_disclosure":disclosure,
            "observation":observation(self.host.publication_for_test())});
        require_native_resident(self.host.publication_for_test(), resident)?;
        let targets = self.visible_targets()?;
        *receipt = json!({"status":"searching","resident":resident,"reselected_before_search":true,
            "live_disclosure":disclosure,
            "visible_hit_tested_targets":targets.sampled,
            "visible_target_search":{
                "strategy":"resolved-non-person-table-sit-priority-fair-spread-v2",
                "ordering":"exact loaded-table Sit first; endpoint-inclusive spread within priority and fallback",
                "metadata_authority":"search hint only; fresh pick and actual offered native popup remain mandatory",
                "excluded_targets":targets.excluded.observation(),
                "table_sit_candidates":targets.sit_candidates,
                "table_sit_discovered":targets.sit_discovered,
                "table_sit_sampled":targets.sit_sampled,
                "on_screen_centroid_object_candidates":targets.candidate_objects,
                "on_screen_centroid_candidates":targets.candidate_centroids,
                "object_candidates_probed":targets.probed_objects,
                "discovered_visible_candidate_count":targets.discovered.len(),
                "discovered_visible_candidates":targets.discovered,
                "sampled_target_count":targets.sampled.len(),
                "discovery_scene_pick_probes":targets.probes,
                "discovery_scene_pick_probe_limit":MAX_HIT_TESTS - MAX_TARGETS,
                "revalidation_scene_pick_probe_reserve":MAX_TARGETS,
                "total_scene_pick_probe_limit":MAX_HIT_TESTS},
            "revalidation_scene_pick_probes":0,"scene_pick_probes":targets.probes,
            "revalidation_excluded_targets":TargetExclusions::default().observation(),
            "attempts":[],"cancellation_completion_verified":false});
        let mut revalidation_probes = 0;
        let mut revalidation_excluded = TargetExclusions::default();
        for (object, pixel) in targets.sampled {
            self.bounded()?;
            let publication = self.host.publication_for_test();
            let eligible = revalidation_excluded
                .priority(object, publication.journey_targets.get(&object))
                .is_some();
            receipt["revalidation_excluded_targets"] = revalidation_excluded.observation();
            if !eligible {
                continue;
            }
            let search_hint = publication.journey_targets.get(&object).cloned();
            let picked = if let Some(scene) = self.host.publication_for_test().scene.as_ref() {
                revalidation_probes += 1;
                receipt["revalidation_scene_pick_probes"] = json!(revalidation_probes);
                receipt["scene_pick_probes"] = json!(targets.probes + revalidation_probes);
                scene.pick(pixel)
            } else {
                None
            };
            if picked != Some(object) {
                continue;
            }
            self.ui(Message::ViewportButton {
                down: true,
                pixel,
                picked: Some(object),
            })?;
            self.ui(Message::ViewportButton {
                down: false,
                pixel,
                picked: Some(object),
            })?;
            self.settle_presentation()?;
            let publication = self.host.publication_for_test().clone();
            receipt["attempts"]
                .as_array_mut()
                .unwrap()
                .push(json!({"object":object,"pixel":pixel,
                "search_hint":search_hint,
                "observation":observation(&publication)}));
            let choice = publication
                .menu
                .as_ref()
                .filter(|menu| menu.ready && menu.popup_visible && !menu.preview)
                .and_then(|menu| {
                    menu.popup.iter().find(|choice| {
                        ["Sit", "Sit Here"].contains(&choice.label.as_str())
                            && matches!(&choice.intent, Some(Intent::ObjectChoice(_)))
                    })
                });
            if let Some(choice) = choice {
                let popup = serde_json::to_value(
                    publication
                        .native_popup
                        .as_ref()
                        .context("offered native popup owner absent")?,
                )?;
                ensure!(
                    popup.pointer("/actor/0").and_then(Value::as_i64) == Some(i64::from(resident)),
                    "offered popup native actor differs from reselected resident"
                );
                let before = serials(&publication);
                receipt["offered_label"] = json!(choice.label);
                receipt["native_popup"] = popup;
                let (publication, added) = self.timed(Scope::NativeProof, Operation::OfferedInteraction, Role::PlayerPath, |journey| {
                    journey.ui(choice.intent.as_ref().unwrap().message())?;
                    journey.settle_presentation()?;
                    let publication = journey.host.publication_for_test().clone();
                    let added = serials(&publication).into_iter()
                        .filter(|serial| !before.contains(serial)).collect::<Vec<_>>();
                    receipt["new_serials"] = json!(added);
                    receipt["enqueue_observation"] = observation(&publication);
                    ensure!(added.len() == 1,
                        "offered Sit did not create one unambiguous observable new action serial");
                    journey.timing_success();
                    Ok((publication, added))
                })?;
                let serial = added[0];
                ensure!(
                    publication.resident == resident,
                    "queue actor changed before cancellation"
                );
                let entry = publication
                    .queue
                    .iter()
                    .find(|entry| u64::from(entry.action.serial) == serial && entry.action.cancel().is_some())
                    .context("new queue action is not available for cancellation")?;
                ensure!(
                    [entry.label.as_str(), entry.diagnostic_label.as_str()]
                        .contains(&choice.label.as_str()),
                    "new action label differs from offered Sit action"
                );
                receipt["status"] = json!("enqueue_observed");
                let continuation = self.timed(
                    Scope::NativeProof,
                    Operation::Cancellation,
                    Role::PlayerPath,
                    |journey| {
                        journey.ui(entry.action.cancel().context("cancellation is already requested")?.message())?;
                        receipt["status"] = json!("cancellation_requested");
                        journey.play("cancel_continuation", None, Some(serial))
                    },
                )?;
                receipt["continuation"] = continuation;
                receipt["cancellation_completion_verified"] = json!(true);
                receipt["status"] = json!("passed");
                return Ok(());
            }
            if let Some(menu) = &publication.menu {
                let cancel = menu
                    .cancel
                    .as_ref()
                    .context("offered menu cannot be closed normally")?;
                self.ui(cancel.message())?;
                self.settle_presentation()?;
            }
        }
        receipt["status"] = json!("unsupported");
        receipt["reason"] = json!(
            "bounded visible hit-tested search offered no native Sit/Sit Here action; no enqueue/cancel coverage"
        );
        Ok(())
    }

    fn save_load(&mut self, output: &Path, save: &Path, receipt: &mut Value) -> Result<()> {
        self.pause()?;
        *receipt =
            json!({"status":"attempted","path":save,"no_frame_between_save_load_resave":true});
        let generation = self.host.current_generation_for_test();
        self.ui_with_role(Message::Save, Role::PrimarySave)?;
        let before = self.timed(
            Scope::EvidenceValidation,
            Operation::ManagedSaveLoad,
            Role::PrimaryEvidenceCopy,
            |_| {
                let before =
                    std::fs::read(save).context("production Host Save produced no bytes")?;
                write_new(&output.join("managed-before.json"), &before)?;
                Ok(before)
            },
        )?;
        let before_publication = observation(self.host.publication_for_test());
        self.timing_checkpoint();
        self.ui_with_role(Message::Save, Role::ControlSave)?;
        let control = self.timed(
            Scope::EvidenceValidation,
            Operation::ManagedSaveLoad,
            Role::ControlEvidenceCopy,
            |_| {
                let control = std::fs::read(save)?;
                write_new(&output.join("managed-save-control.json"), &control)?;
                Ok(control)
            },
        )?;
        ensure!(
            self.host.current_generation_for_test() == generation,
            "Save-only control unexpectedly replaced the Host owner"
        );
        self.timing_checkpoint();
        self.timed(
            Scope::EvidenceValidation,
            Operation::ManagedSaveLoad,
            Role::InputReselection,
            |_| {
                super::super::super::atomic_save(save, &before)?;
                ensure!(
                    std::fs::read(save)? == before,
                    "Load input differs from the first exact own-save bytes"
                );
                Ok(())
            },
        )?;
        self.ui_with_role(Message::Load, Role::ManagedRestore)?;
        let restored_generation = self.host.current_generation_for_test();
        ensure!(
            restored_generation
                == generation
                    .checked_add(1)
                    .context("Host generation overflow")?,
            "managed Load did not advance real Host generation once"
        );
        self.timing_checkpoint();
        self.ui_with_role(Message::Save, Role::ValidationResave)?;
        let after = self.timed(
            Scope::EvidenceValidation,
            Operation::ManagedSaveLoad,
            Role::ResaveEvidenceCopy,
            |_| {
                let after = std::fs::read(save)?;
                write_new(&output.join("managed-frozen-resave.json"), &after)?;
                Ok(after)
            },
        )?;
        self.timed(Scope::EvidenceValidation, Operation::ManagedSaveLoad, Role::HashAndFullStateValidation, |journey| {
        let expected: Value = serde_json::from_slice(&before)?;
        let actual: Value = serde_json::from_slice(&after)?;
        let control_state: Value = serde_json::from_slice(&control)?;
        ensure!(
            expected["session"]["version"] == 2
                && actual["session"]["version"] == 2
                && control_state["session"]["version"] == 2,
            "Host save is not managed version 2"
        );
        let vm = |value: &Value| -> Result<Value> {
            let vm = value
                .pointer("/session/simulation/state/vm")
                .context("managed VM state absent")?;
            ensure!(
                vm.get("tick").is_some() && vm.get("rng_state").is_some(),
                "managed tick/RNG evidence absent"
            );
            Ok(
                json!({"tick":vm["tick"],"rng_state":vm["rng_state"],"lot_rng_state":vm["lot_rng_state"],
                "crt_rng_state":vm["crt_rng_state"],"lot_rng_scope":vm["lot_rng_scope"]}),
            )
        };
        *receipt = json!({"status":"compared","before_generation":generation,"restored_generation":restored_generation,
            "before_sha256":engine::digest(&before),"after_sha256":engine::digest(&after),
            "bytes_equal":before==after,"full_state_equal":expected==actual,
            "control_sha256":engine::digest(&control),
            "control_bytes_equal":control==after,"control_full_state_equal":control_state==actual,
            "difference_paths":managed_difference_paths(&expected,&actual),
            "session_state_equal":expected["session"]==actual["session"],
            "before_tick_rng":vm(&expected)?,"after_tick_rng":vm(&actual)?,
            "before":before_publication,"after":observation(journey.host.publication_for_test()),
            "no_frame_between_save_load_resave":true,"normalizations":[],"own_save_only":true,
            "control_path":"same Host Save -> Save, then byte-exact first own-save file selected for Load -> Save",
            "load_input_sha256":engine::digest(&before),"load_input_reselected_from":"managed-before.json",
            "qualification":"exact managed continuation and complete matched-control snapshot; accounted camera republication, not first-file byte equality"});
        receipt["control_republication"] =
            accounted_camera_republication(&expected, &control_state)?;
        receipt["restored_republication"] = accounted_camera_republication(&expected, &actual)?;
        ensure!(
            control_state == actual,
            "full restored state differs from the production Save-only control; raw evidence preserved"
        );
        ensure!(
            control == after,
            "restored re-save bytes differ from the production Save-only control; raw evidence preserved"
        );
        receipt["status"] = json!("passed");
        Ok(())
        })?;
        self.settle_presentation()
    }
}

fn write_new(path: &Path, bytes: &[u8]) -> Result<()> {
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?
        .write_all(bytes)?;
    Ok(())
}

fn publish_journey_diagnostics(
    result: Result<()>,
    report: &mut Value,
    timing_export: Result<Option<Value>>,
    export_report: impl FnOnce(&Value) -> Result<()>,
) -> Result<()> {
    match &timing_export {
        Ok(Some(evidence)) => report["timing"] = evidence.clone(),
        Ok(None) => {}
        Err(error) => {
            let message = format!("{error:#}");
            report["timing_export_error"] = json!(message);
            report["timing"] = json!({"path":"timing.json","complete":false,"export_failed":true});
            report["success"] = json!(false);
            if report["first_error"].is_null() {
                report["first_error"] = json!(message);
                if report.get("first_blocker").is_none() {
                    report["first_blocker"] = json!({"stage":"timing_export","error":message});
                }
            }
            let _ = writeln!(std::io::stderr().lock(), "Host journey timing export failed: {error:#}");
        }
    }
    let report_export = export_report(report);
    if let Err(error) = &report_export {
        let _ = writeln!(std::io::stderr().lock(), "Host journey report export failed: {error:#}");
    }
    result?;
    timing_export.map(|_| ())?;
    report_export
}

#[test]
fn diagnostic_export_success_publishes_timing_once() -> Result<()> {
    let mut report = json!({"success":true,"first_error":null});
    let timing = json!({"path":"timing.json","complete":true,"truncated":false});
    let writes = std::cell::Cell::new(0);
    publish_journey_diagnostics(Ok(()), &mut report, Ok(Some(timing.clone())), |report| {
        writes.set(writes.get() + 1);
        assert_eq!(report["timing"], timing);
        assert_eq!(report["success"], true);
        Ok(())
    })?;
    assert_eq!(writes.get(), 1);
    assert!(report.get("timing_export_error").is_none());
    Ok(())
}

#[test]
fn diagnostic_export_disabled_preserves_the_report() -> Result<()> {
    let original = json!({"success":false,"first_error":"existing","first_blocker":{"stage":"game"}});
    let mut report = original.clone();
    publish_journey_diagnostics(Ok(()), &mut report, Ok(None), |report| {
        assert_eq!(report, &original);
        Ok(())
    })?;
    assert_eq!(report, original);
    Ok(())
}

#[test]
fn diagnostic_export_failure_on_game_success_denies_success_and_writes_report() {
    let mut report = json!({"success":true,"first_error":null});
    let writes = std::cell::Cell::new(0);
    let failure = publish_journey_diagnostics(
        Ok(()),
        &mut report,
        Err(anyhow::anyhow!("timing write failed")),
        |report| {
            writes.set(writes.get() + 1);
            assert_eq!(report["success"], false);
            assert_eq!(report["timing"]["complete"], false);
            assert_eq!(report["timing"]["export_failed"], true);
            assert_eq!(report["first_blocker"]["stage"], "timing_export");
            assert_eq!(report["timing_export_error"], "timing write failed");
            Ok(())
        },
    ).unwrap_err();
    assert_eq!(failure.to_string(), "timing write failed");
    assert_eq!(writes.get(), 1);
}

#[test]
fn diagnostic_export_failure_preserves_the_original_game_error_and_chain() {
    let game_error = anyhow::Error::new(std::io::Error::other("original game fault"))
        .context("original game context");
    let original_pointer = game_error.downcast_ref::<std::io::Error>().unwrap() as *const std::io::Error;
    let original_chain = format!("{game_error:#}");
    let mut report = json!({"success":false,"first_error":original_chain,
        "first_blocker":{"stage":"play","error":original_chain}});
    let writes = std::cell::Cell::new(0);
    let failure = publish_journey_diagnostics(
        Err(game_error),
        &mut report,
        Err(anyhow::anyhow!("timing serialization failed")),
        |report| {
            writes.set(writes.get() + 1);
            assert_eq!(report["first_error"], original_chain);
            assert_eq!(report["first_blocker"]["stage"], "play");
            assert_eq!(report["timing_export_error"], "timing serialization failed");
            Ok(())
        },
    ).unwrap_err();
    assert_eq!(failure.downcast_ref::<std::io::Error>().unwrap() as *const std::io::Error, original_pointer);
    assert_eq!(format!("{failure:#}"), original_chain);
    assert_eq!(writes.get(), 1);
}

#[test]
fn diagnostic_export_report_failure_preserves_the_original_game_error() {
    let game_error = anyhow::Error::new(std::io::Error::other("original game fault"))
        .context("original game context");
    let original_pointer = game_error.downcast_ref::<std::io::Error>().unwrap() as *const std::io::Error;
    let original_chain = format!("{game_error:#}");
    let mut report = json!({"success":false,"first_error":original_chain});
    let failure = publish_journey_diagnostics(
        Err(game_error),
        &mut report,
        Ok(None),
        |_| Err(anyhow::anyhow!("report write failed")),
    ).unwrap_err();
    assert_eq!(failure.downcast_ref::<std::io::Error>().unwrap() as *const std::io::Error, original_pointer);
    assert_eq!(format!("{failure:#}"), original_chain);
}

#[test]
fn diagnostic_export_both_export_failures_preserve_the_original_game_error() {
    let game_error = anyhow::Error::new(std::io::Error::other("original game fault"));
    let original_pointer = game_error.downcast_ref::<std::io::Error>().unwrap() as *const std::io::Error;
    let mut report = json!({"success":false,"first_error":"original game fault"});
    let writes = std::cell::Cell::new(0);
    let failure = publish_journey_diagnostics(
        Err(game_error),
        &mut report,
        Err(anyhow::anyhow!("timing write failed")),
        |_| {
            writes.set(writes.get() + 1);
            Err(anyhow::anyhow!("report write failed"))
        },
    ).unwrap_err();
    assert_eq!(failure.downcast_ref::<std::io::Error>().unwrap() as *const std::io::Error, original_pointer);
    assert_eq!(writes.get(), 1);
}

#[test]
fn diagnostic_export_game_success_returns_report_failure_without_timing_failure() {
    let mut report = json!({"success":true,"first_error":null});
    let failure = publish_journey_diagnostics(
        Ok(()),
        &mut report,
        Ok(None),
        |_| Err(anyhow::anyhow!("report write failed")),
    ).unwrap_err();
    assert_eq!(failure.to_string(), "report write failed");
}

#[test]
fn diagnostic_export_game_success_keeps_timing_failure_primary_over_report_failure() {
    let mut report = json!({"success":true,"first_error":null});
    let failure = publish_journey_diagnostics(
        Ok(()),
        &mut report,
        Err(anyhow::anyhow!("timing write failed")),
        |_| Err(anyhow::anyhow!("report write failed")),
    ).unwrap_err();
    assert_eq!(failure.to_string(), "timing write failed");
    assert_eq!(report["success"], false);
}

fn env_bool(name: &str) -> Result<bool> {
    env_bool_with(name, |name| std::env::var(name))
}

fn env_bool_with(
    name: &str,
    read: impl FnOnce(&str) -> std::result::Result<String, std::env::VarError>,
) -> Result<bool> {
    match read(name).as_deref() {
        Ok("1") => Ok(true),
        Ok("0") | Err(std::env::VarError::NotPresent) => Ok(false),
        _ => anyhow::bail!("{name} must be 0 or 1"),
    }
}

fn object_popup_release_enabled_with(
    mut read: impl FnMut(&str) -> std::result::Result<String, std::env::VarError>,
) -> Result<bool> {
    let enabled = env_bool_with("SIMS_HOST_JOURNEY_TUTORIAL_OBJECT_POPUP_RELEASE", &mut read)?;
    if !enabled {
        return Ok(false);
    }
    ensure!(
        env_bool_with("SIMS_HOST_JOURNEY_TUTORIAL_OBJECT_INTERACTION", &mut read)?,
        "tutorial object popup release requires OBJECT_INTERACTION=1"
    );
    tutorial_camera::object_dependencies(
        env_bool_with("SIMS_HOST_JOURNEY_TUTORIAL_CAMERA", &mut read)?,
        env_bool_with("SIMS_HOST_JOURNEY_TUTORIAL_GO_HERE", &mut read)?,
        env_bool_with("SIMS_HOST_JOURNEY_TUTORIAL_WALKING", &mut read)?,
        &read("SIMS_HOST_JOURNEY_SOURCE")?,
    )?;
    Ok(true)
}

#[test]
fn timing_flag_absent_and_zero_use_the_disabled_recorder_path() -> Result<()> {
    for value in [None, Some("0")] {
        let enabled = env_bool_with("SIMS_HOST_JOURNEY_TIMING", |_| {
            value
                .map(str::to_owned)
                .ok_or(std::env::VarError::NotPresent)
        })?;
        assert!(!enabled);
        assert!(Recorder::enabled(enabled, MAX_COMMANDS).is_none());
    }
    assert!(env_bool_with("SIMS_HOST_JOURNEY_TIMING", |_| Ok(
        "1".to_owned()
    ))?);
    Ok(())
}

#[test]
fn invalid_timing_flag_fails_before_the_construction_branch_without_mutating_environment() {
    for value in ["", "true", "01", " 1", "2"] {
        let constructed = std::cell::Cell::new(false);
        let result = (|| -> Result<()> {
            env_bool_with("SIMS_HOST_JOURNEY_TIMING", |_| Ok(value.to_owned()))?;
            constructed.set(true);
            Ok(())
        })();
        assert!(result.is_err());
        assert!(!constructed.get());
    }
    assert!(
        env_bool_with("SIMS_HOST_JOURNEY_TIMING", |_| Err(
            std::env::VarError::NotUnicode("invalid".into())
        ))
        .is_err()
    );
}

fn terminal(viewer: Viewer, output: &Path) -> Result<Value> {
    let session = viewer
        .session
        .as_ref()
        .context("terminal Host session absent")?;
    let native_selection = session.simulation.native_selected_person();
    let client = viewer
        .client
        .as_ref()
        .context("terminal dialog client absent")
        .and_then(|client| client.save(session));
    let device = viewer
        .device
        .as_ref()
        .map(|device| device.save(session))
        .transpose();
    let summary = json!({"tick":session.simulation.vm.tick,
        "rng":[session.simulation.vm.rng_state,session.simulation.vm.lot_rng_state,session.simulation.vm.crt_rng_state],
        "lot_rng_scope":session.simulation.vm.lot_rng_scope,
        "native_selected_person":native_selection.as_ref().ok(),
        "native_selection_error":native_selection.as_ref().err(),
        "resident":viewer.resident,"selected":viewer.selected,"error":viewer.error,
        "menu_error":viewer.menu_error,"configuration":viewer.application});
    let state = json!({"session":session.save(),
        "client":client.as_ref().ok(),"client_error":client.as_ref().err().map(|error| format!("{error:#}")),
        "device":device.as_ref().ok(),"device_error":device.as_ref().err().map(|error| format!("{error:#}"))});
    let bytes = serde_json::to_vec(&state)?;
    write_new(&output.join("terminal-continuation.json"), &bytes)?;
    Ok(
        json!({"summary":summary,"state_sha256":engine::digest(&bytes),
        "complete":client.is_ok() && device.is_ok(),"owner_recovered_only_at_terminal":true}),
    )
}

fn run() -> Result<()> {
    let object_progress_enabled =
        object_popup_release_enabled_with(|name| std::env::var(name))?;
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    let relative = PathBuf::from(std::env::var("SIMS_HOST_JOURNEY_OUTPUT")?);
    ensure!(
        relative.starts_with("cache")
            && relative.components().count() > 1
            && relative
                .components()
                .all(|component| matches!(component, std::path::Component::Normal(_))),
        "Host output must be a new directory under cache"
    );
    let output = root.join(relative);
    std::fs::create_dir(&output).context("refusing to overwrite Host journey evidence")?;
    let wall = Instant::now();
    let mut driver = None;
    let mut stage = "configuration";
    let mut report = json!({"success":false,"harness":"continuous-real-host-player-journey-v1",
        "operations":{"fresh_async_load":{"status":"not_reached"},"initial_play":{"status":"not_reached"},
            "resident_selection":{"status":"not_reached"},"buy_live":{"status":"not_reached"},
            "interaction_queue_cancel":{"status":"not_reached"},"managed_save_load":{"status":"not_reached"},
            "post_reload_play":{"status":"not_reached"}},
        "continuous_host":true,"direct_install":false,"input_save_restored":false,
        "per_frame_audit":false,"injected_gameplay_state":false,"strict_original_parity_ticks":0,
        "window_qualified":false,"fps_qualified":false,"full_milestone_qualified":false});
    let result = (|| -> Result<()> {
        let timing_enabled = env_bool("SIMS_HOST_JOURNEY_TIMING")?;
        let source = std::env::var("SIMS_HOST_JOURNEY_SOURCE")
            .context("explicit household source required")?;
        let source_path = Path::new(&source);
        ensure!(
            !source_path.is_absolute()
                && source_path
                    .components()
                    .all(|component| matches!(component, std::path::Component::Normal(_))),
            "household source must be asset-root-relative"
        );
        if env_bool("SIMS_HOST_JOURNEY_TUTORIAL_OBJECT_INTERACTION")? {
            tutorial_camera::object_dependencies(
                env_bool("SIMS_HOST_JOURNEY_TUTORIAL_CAMERA")?,
                env_bool("SIMS_HOST_JOURNEY_TUTORIAL_GO_HERE")?,
                env_bool("SIMS_HOST_JOURNEY_TUTORIAL_WALKING")?,
                &source,
            )?;
        }
        let assets = std::env::var_os("SIMS_HOST_JOURNEY_ASSETS")
            .map(PathBuf::from)
            .unwrap_or_else(|| root.join("game-data/cabinet-extracted"));
        let save = output.join("player-save.json");
        let mut args = Args::try_parse_from([
            "sims-room",
            "--original",
            source.as_str(),
            "--root",
            assets.to_str().context("asset path must be UTF-8")?,
            "--paused",
            "--floor",
            "1",
            "--save",
            save.to_str().context("save path must be UTF-8")?,
        ])?;
        args.inferred_routing = env_bool("SIMS_HOST_JOURNEY_INFERRED_ROUTING")?;
        args.buy_preview = env_bool("SIMS_HOST_JOURNEY_BUY_PREVIEW")?;
        let tutorial_camera = env_bool("SIMS_HOST_JOURNEY_TUTORIAL_CAMERA")?;
        if tutorial_camera {
            ensure!(
                source == "UserData/Houses/House07.iff",
                "tutorial camera requires fresh Newbie source"
            );
            report["configuration_tutorial_camera"] = json!({"enabled":true,
                "frames_per_expected_transition":256,"total_frames_including_service_polls":2048,
                "scope":"camera -> instructed Live -> visible enabled Bob; not Betty or tutorial completion"});
        }
        args.neighborhood_policy = match std::env::var("SIMS_HOST_JOURNEY_NEIGHBORHOOD_POLICY") {
            Ok(policy) => {
                ensure!(
                    policy == super::super::super::neighborhood_usability::POLICY,
                    "unsupported Host neighborhood policy"
                );
                Some(policy)
            }
            Err(std::env::VarError::NotPresent) => None,
            Err(error) => return Err(error.into()),
        };
        let minutes = std::env::var("SIMS_HOST_JOURNEY_CLOCK_MINUTES")
            .unwrap_or_else(|_| "1".into())
            .parse::<i64>()?;
        let wall_seconds = std::env::var("SIMS_HOST_JOURNEY_WALL_SECONDS")
            .unwrap_or_else(|_| "420".into())
            .parse::<f64>()?;
        ensure!(
            (1..=60).contains(&minutes) && (1.0..=900.0).contains(&wall_seconds),
            "Host journey limits out of range"
        );
        ensure!(
            args.load.is_none() && args.at_tick == 0 && !save.exists(),
            "Host journey must load fresh and own a new save"
        );
        report["configuration"] = json!({"source":source,"assets":assets,"buy_preview":args.buy_preview,
            "inferred_routing":args.inferred_routing,"neighborhood_policy":args.neighborhood_policy,
            "gameplay_changing_options":"all requested options are replacement/unverified behavior, not original parity",
            "installed_slots":"positive installed vacant/reserved slots are intentional; no inferred proximity/order claim"});
        report["limits"] = json!({"commands_including_task_outputs":MAX_COMMANDS,"frames":MAX_FRAMES,
            "service_polls":MAX_POLLS,"effect_polls":MAX_POLLS,"task_streams":MAX_COMMANDS,
            "completed_ticks_per_play_phase":MAX_TICKS,"targets":MAX_TARGETS,"hit_tests":MAX_HIT_TESTS,
            "hit_tests_scope":"discovery plus live-scene target revalidation; discovery reserves one pick per possible target",
            "dialogs":16,"wall_seconds":wall_seconds,"target_clock_minute_boundaries":minutes,
            "wall_semantics":"timed stream delivery; native synchronous calls/settle/terminal capture are non-preemptible; external deadline required"});
        let executor = ThreadPool::builder()
            .pool_size(2)
            .stack_size(64 * 1024 * 1024)
            .create()?;
        let (sender, receiver) = mpsc::channel();
        stage = "fresh_async_load";
        report["operations"][stage] = json!({"status":"attempted"});
        let timing = Recorder::enabled(timing_enabled, MAX_COMMANDS);
        let fresh = timing.as_ref().and_then(|timing| {
            timing.start(
                None,
                Scope::ConstructionToSettled,
                Operation::FreshLoad,
                Role::PlayerPath,
                None,
                true,
            )
        });
        let (host, task) = if object_progress_enabled {
            Host::new_with_object_progress_for_test(
                args, timing.as_ref().map(|timing| timing.loader_probe(fresh)),
            )
        } else {
            match &timing {
                Some(timing) => Host::new_with_loader_probe(args, timing.loader_probe(fresh)),
                None => Host::new(args),
            }
        };
        driver = Some(Journey {
            host,
            executor,
            sender,
            receiver,
            streams: 0,
            tasks: 0,
            idle: false,
            wall,
            wall_seconds,
            epoch: Instant::now(),
            millis: 0,
            commands: 0,
            frames: 0,
            service_polls: 0,
            effect_polls: 0,
            dialogs: 0,
            events: vec![],
            timing,
            timing_parent: None,
        });
        let journey = driver.as_mut().unwrap();
        let loaded = journey.drive(task);
        if let Some(timing) = &journey.timing {
            let returned = timing.timestamp();
            timing.drive_return_at(fresh, returned, journey.timing_observation().owner_drained);
            if let Err(error) = &loaded {
                timing.finish(
                    fresh,
                    Outcome::Error,
                    Some(journey.timing_observation()),
                    Some(&format!("{error:#}")),
                );
            }
        }
        loaded?;
        ensure!(
            journey.host.publication_for_test().session_present,
            "actual Loaded effect installed no household"
        );
        journey.timing_checkpoint();
        report["operations"][stage] = json!({"status":"passed","observation":observation(journey.host.publication_for_test()),
            "generation":journey.host.current_generation_for_test(),"path":"Host::new -> actual Iced Loaded effect -> worker"});
        if tutorial_camera {
            stage = "tutorial_camera_live_bob";
            report["operations"][stage] = json!({"status":"attempted"});
            let camera = journey.timed(
                Scope::JourneyOperation,
                Operation::TutorialCamera,
                Role::PlayerPath,
                tutorial_camera::run,
            );
            report["operations"][stage] = match &camera {
                Ok(receipt) => receipt.clone(),
                Err(error) => json!({"status":"unsupported","reason":format!("{error:#}")}),
            };
            camera?;
        }
        stage = "initial_play";
        report["operations"][stage] = json!({"status":"attempted"});
        report["operations"][stage] = journey.timed(
            Scope::JourneyOperation,
            Operation::InitialPlay,
            Role::PlayerPath,
            |journey| journey.play(stage, Some(minutes), None),
        )?;
        stage = "resident_selection";
        report["operations"][stage] = json!({"status":"attempted"});
        report["operations"][stage] = journey.timed(
            Scope::JourneyOperation,
            Operation::ResidentSelection,
            Role::PlayerPath,
            Journey::residents,
        )?;
        stage = "buy_live";
        report["operations"][stage] = json!({"status":"attempted"});
        report["operations"][stage] = journey.timed(
            Scope::JourneyOperation,
            Operation::BuyLive,
            Role::PlayerPath,
            Journey::buy_live,
        )?;
        if report["operations"][stage]["status"] == "unsupported" {
            report["first_blocker"] =
                json!({"stage":stage,"operation":report["operations"][stage]});
        }
        stage = "interaction_queue_cancel";
        report["operations"][stage] = json!({"status":"attempted"});
        journey.timed(
            Scope::JourneyOperation,
            Operation::InteractionQueueCancel,
            Role::PlayerPath,
            |journey| {
                journey.interaction(&mut report["operations"][stage])?;
                Ok(if journey.timing.is_some() {
                    report["operations"][stage].clone()
                } else {
                    Value::Null
                })
            },
        )?;
        if report["operations"][stage]["status"] == "unsupported"
            && report.get("first_blocker").is_none()
        {
            report["first_blocker"] =
                json!({"stage":stage,"operation":report["operations"][stage]});
        }
        stage = "managed_save_load";
        journey.timed(
            Scope::JourneyOperation,
            Operation::ManagedSaveLoad,
            Role::PlayerPath,
            |journey| journey.save_load(&output, &save, &mut report["operations"][stage]),
        )?;
        stage = "post_reload_play";
        report["operations"][stage] = json!({"status":"attempted"});
        report["operations"][stage] = journey.timed(
            Scope::JourneyOperation,
            Operation::PostReloadPlay,
            Role::PlayerPath,
            |journey| journey.play(stage, Some(minutes), None),
        )?;
        stage = "complete";
        Ok(())
    })();
    report["stage"] = json!(stage);
    report["bounded_journey_completed"] = json!(result.is_ok());
    report["first_error"] = json!(result.as_ref().err().map(|error| format!("{error:#}")));
    if let Err(error) = &result {
        if report.get("first_blocker").is_none() {
            report["first_blocker"] = json!({"stage":stage,"error":format!("{error:#}")});
        }
    }
    let timing = driver.as_ref().and_then(|journey| journey.timing.clone());
    if let Some(journey) = driver {
        report["final_publication"] = observation(journey.host.publication_for_test());
        report["driver"] = json!({"commands":journey.commands,"frames":journey.frames,"service_polls":journey.service_polls,
            "effect_polls":journey.effect_polls,"task_streams":journey.tasks,"unfinished_streams":journey.streams,
            "idle":journey.idle,"dialogs":journey.dialogs,"generation":journey.host.current_generation_for_test()});
        write_new(
            &output.join("events.json"),
            &serde_json::to_vec_pretty(&journey.events)?,
        )?;
        if journey.idle {
            match terminal(journey.host.recover_for_test(), &output) {
                Ok(evidence) => {
                    if evidence["complete"] != true {
                        report["terminal_capture_error"] = json!(
                            "terminal component capture incomplete; retained full session and component errors"
                        );
                    }
                    report["terminal"] = evidence;
                }
                Err(error) => report["terminal_capture_error"] = json!(format!("{error:#}")),
            }
        } else {
            report["terminal_capture_skipped"] = json!(
                "owner/effects not proven idle; no blocking recovery and no completion credit"
            );
        }
    }
    report["wall_seconds"] = json!(wall.elapsed().as_secs_f64());
    if report["first_error"].is_null() {
        if let Some(error) = report.get("terminal_capture_error").cloned() {
            report["first_error"] = error.clone();
            if report.get("first_blocker").is_none() {
                report["first_blocker"] = json!({"stage":"terminal_evidence","error":error});
            }
        }
    }
    let qualified = report["operations"]
        .as_object()
        .unwrap()
        .values()
        .all(|operation| operation["status"] == "passed");
    report["all_operations_qualified"] = json!(qualified);
    if result.is_ok() && !qualified && report["first_error"].is_null() {
        report["first_error"] = json!(
            "bounded journey completed with unsupported/unqualified operations; no missing coverage credit"
        );
    }
    report["success"] =
        json!(result.is_ok() && qualified && report.get("terminal_capture_error").is_none());
    let timing_export = (|| -> Result<Option<Value>> {
        let Some(timing) = timing else {
            return Ok(None);
        };
        let artifact = timing.try_final_artifact(
            report["success"] == true,
            report["terminal"]["complete"] == true
                && report.get("terminal_capture_error").is_none()
                && report.get("terminal_capture_skipped").is_none(),
        )?;
        write_new(
            &output.join("timing.json"),
            &serde_json::to_vec_pretty(&artifact)?,
        )?;
        Ok(Some(json!({"path":"timing.json","truncated":artifact["truncated"],
            "complete":artifact["complete"]})))
    })();
    publish_journey_diagnostics(result, &mut report, timing_export, |report| {
        write_new(
            &output.join("report.json"),
            &serde_json::to_vec_pretty(report)?,
        )
    })?;
    ensure!(
        qualified,
        "continuous Host journey has unsupported/unqualified operations; see report.json"
    );
    ensure!(
        report.get("terminal_capture_error").is_none(),
        "terminal evidence capture failed; see report.json"
    );
    Ok(())
}


#[test]
fn loader_stage_recording_export_failure_preserves_original_game_chain_and_attempts_report() {
    for report_fails in [false, true] {
        let recorder = Recorder::enabled(true, 0).unwrap();
        let original = anyhow::anyhow!("original loader game failure").context("loader game context");
        let identity = original.root_cause() as *const dyn std::error::Error;
        let game = recorder.loader_probe(None).measure_staged(|stages| {
            player_timing::LoaderStages::enter_optional(&mut Some(stages), player_timing::LoaderPhase::IndexScanRoute);
            Err::<(), _>(original)
        });
        let mut report = json!({"success":false,"first_error":"original loader game failure"});
        let writes = std::cell::Cell::new(0);
        let export = recorder.try_final_artifact(false, false).map(Some);
        assert!(export.is_err());
        let returned = publish_journey_diagnostics(game, &mut report, export, |_| {
            writes.set(writes.get() + 1);
            if report_fails { anyhow::bail!("secondary report failure"); }
            Ok(())
        }).unwrap_err();
        assert!(std::ptr::eq(returned.root_cause() as *const dyn std::error::Error, identity));
        assert_eq!(format!("{returned:#}"), "loader game context: original loader game failure");
        assert_eq!(writes.get(), 1);
        assert_eq!(report["success"], false);
        assert_eq!(report["timing"]["export_failed"], true);
    }
}
