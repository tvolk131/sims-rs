//! Automated Complete Collection interactive object sweep and live special-panel
//! verification (`Lot::pie_menu` + `Lot::pick` + strict stepping).
//!
//! All live-install tests skip cleanly when `SIMS_INSTALL` is unset.
//! Set `SIMS_SWEEP_REPORT=/path/to/report.md` when running the full sweep to
//! write a ranked Markdown breakdown of every distinct `Trap::Unsupported` hit.

use sims_sim::World;
use sims_vm::boot::{Frames, Lot, StepError};
use sims_vm::play::Play;
use sims_vm::{
    Answer, Interpreter, Player, Shown, ShownSpecialPanel, SpecialPanelRequest,
    SpecialPanelResponse, SpecialPanels, Stream, TickError, Trap, selected,
};
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::PathBuf;

fn sims_install() -> Option<PathBuf> {
    std::env::var_os("SIMS_INSTALL").map(PathBuf::from)
}

/// Player used during headless interaction sweeps: accepts ordinary dialogs
/// with their first offered button (supplying `"Sim"` for kind-3 text prompts)
/// and answers every special panel (`5..=15`) with a realistic valid selection.
struct SweepPlayer {
    neighbor_pick: i16,
    panels_seen: Vec<SpecialPanelRequest>,
}

impl SweepPlayer {
    fn new(world: &World) -> Self {
        let neighbor_pick = world
            .neighbors
            .slots
            .iter()
            .flatten()
            .next()
            .map_or(1, |r| r.id.max(1));
        Self {
            neighbor_pick,
            panels_seen: Vec::new(),
        }
    }
}

impl Player for SweepPlayer {
    fn answer(&mut self, shown: &Shown) -> Option<Answer> {
        let status = *shown.answers.first()?;
        Some(Answer {
            status,
            bitmap_id: None,
            text: shown.text_entry.then(|| "Sim".to_owned()),
        })
    }

    fn special_panel(&mut self, shown: &ShownSpecialPanel) -> Option<SpecialPanelResponse> {
        self.panels_seen.push(shown.request.clone());
        Some(match shown.request {
            SpecialPanelRequest::DowntownLotPicker => SpecialPanelResponse::Lot(Some(21)),
            SpecialPanelRequest::VacationLotPicker { .. } => SpecialPanelResponse::Lot(Some(40)),
            SpecialPanelRequest::OldTownLotPicker { .. } => SpecialPanelResponse::Lot(Some(58)),
            SpecialPanelRequest::StudioTownLotPicker { .. } => SpecialPanelResponse::Lot(Some(81)),
            SpecialPanelRequest::MagicTownLotPicker { .. } => SpecialPanelResponse::Lot(Some(93)),
            SpecialPanelRequest::CallNeighbor { .. } => {
                SpecialPanelResponse::Neighbor(Some(self.neighbor_pick))
            }
            SpecialPanelRequest::WardrobeShop { .. }
            | SpecialPanelRequest::PetPicker { .. }
            | SpecialPanelRequest::CharmTransform { .. } => SpecialPanelResponse::Selection {
                accepted: true,
                index: 0,
            },
            SpecialPanelRequest::SpellBook { .. } | SpecialPanelRequest::CookBook { .. } => {
                SpecialPanelResponse::Dismissed
            }
        })
    }
}

fn definition_and_ctss(lot: &Lot, guid: u32) -> Option<(usize, &[i16; 108], u16)> {
    let entry = lot.catalog().object(guid)?;
    let lib = lot.program().libraries.get(&entry.resource)?;
    let words = lib.definitions.get(&entry.chunk)?;
    let ctss = words.get(41).copied().unwrap_or(0) as u16;
    if ctss != 0 {
        return Some((entry.resource, words, ctss));
    }
    // Multi-tile lead tiles keep price & CTSS on their master OBJD in the same library.
    for &chunk in &lib.definition_order {
        if let Some(w) = lib.definitions.get(&chunk) {
            let c = w.get(41).copied().unwrap_or(0) as u16;
            if c != 0 {
                return Some((entry.resource, w, c));
            }
        }
    }
    Some((entry.resource, words, 0))
}

fn object_label(lot: &Lot, id: u16) -> String {
    let Some(obj) = lot.world().objects.get(&id) else {
        return format!("object#{id}");
    };
    let Some((resource, _words, ctss)) = definition_and_ctss(lot, obj.guid) else {
        return format!("GUID 0x{:08X}", obj.guid);
    };
    let off_world = obj.position == [0, 0, 0];
    let tag = if off_world { " [off-world]" } else { "" };
    if let Some(lib) = lot.program().libraries.get(&resource)
        && let Some(entries) = lib.catalog_strings.get(&ctss)
        && let Some((_, name)) = entries
            .iter()
            .find(|(lang, text)| (*lang <= 1) && !text.is_empty())
            .or_else(|| entries.iter().find(|(_, text)| !text.is_empty()))
    {
        return format!("{name} [res#{resource}]{tag} (0x{:08X})", obj.guid);
    }
    format!("[res#{resource}]{tag} (0x{:08X})", obj.guid)
}

/// Whether `id` is a user-placeable/buyable catalog object (non-zero catalog
/// sort / price on any OBJD in its library, placed on the lot, not an invisible controller).
fn is_catalog_object(lot: &Lot, id: u16) -> bool {
    let Some(obj) = lot.world().objects.get(&id) else {
        return false;
    };
    if obj.position == [0, 0, 0] {
        return false;
    }
    let Some(entry) = lot.catalog().object(obj.guid) else {
        return false;
    };
    if entry.kind != 4 {
        return false;
    }
    let Some((resource, words, ctss)) = definition_and_ctss(lot, obj.guid) else {
        return false;
    };
    if ctss == 0 {
        return false;
    }
    // Check master/sibling OBJD for price or catalog sort (word 18 = price, word 40 = catalog sort).
    if words.get(18).copied().unwrap_or(0) > 0 || words.get(40).copied().unwrap_or(0) != 0 {
        return true;
    }
    lot.program().libraries.get(&resource).is_some_and(|lib| {
        lib.definitions
            .values()
            .any(|w| w.get(18).copied().unwrap_or(0) > 0 || w.get(40).copied().unwrap_or(0) != 0)
    })
}

fn menu_entry_label(entry: &sims_vm::orders::MenuEntry) -> String {
    if let Some(name) = &entry.name
        && !name.is_empty()
    {
        return name.clone();
    }
    format!(
        "menu#{} (tree 0x{:04X})",
        entry.action.table_index, entry.action.action
    )
}

fn find_actor(world: &World) -> Option<u16> {
    if let Some(sim) = selected(world) {
        return Some(sim);
    }
    // Prefer an adult/child human resident/person (person.data[65] <= 1).
    world
        .people
        .iter()
        .copied()
        .find(|id| {
            world
                .objects
                .get(id)
                .and_then(|o| o.person.as_ref())
                .is_some_and(|p| p.data.get(65).copied().unwrap_or(0) <= 1)
        })
        .or_else(|| world.people.first().copied())
}

#[derive(Default)]
struct UnsupportedBucket {
    count: usize,
    samples: Vec<String>,
    lots: BTreeSet<String>,
}

#[derive(Default)]
struct LotSweepStats {
    lot_name: String,
    objects_scanned: usize,
    pie_menu_unsupported: usize,
    entries_tested: usize,
    clean: usize,
    native_faults: usize,
    invalid_faults: usize,
    unsupported_hits: usize,
    special_panels_triggered: BTreeMap<u8, usize>,
    warmup_errors: Vec<String>,
    skipped_reason: Option<String>,
}

/// Normalizes dynamic object IDs in `Trap::Unsupported` messages so identical
/// root causes group together cleanly.
fn normalize_unsupported(msg: &str) -> String {
    let mut out = String::with_capacity(msg.len());
    let bytes = msg.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        // Collapse `object 123` / `person 123` numeric IDs to `<id>`.
        if (msg[i..].starts_with("object ") || msg[i..].starts_with("person "))
            && bytes.get(i + 7).is_some_and(|b| b.is_ascii_digit())
        {
            out.push_str(&msg[i..i + 7]);
            out.push_str("<id>");
            i += 7;
            while i < bytes.len() && bytes[i].is_ascii_digit() {
                i += 1;
            }
            continue;
        }
        let ch = msg[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

fn sweep_one_lot(
    root: &std::path::Path,
    label: &str,
    rel_house: &str,
    select_actor: bool,
    ticks_per_interaction: u32,
    buckets: &mut BTreeMap<String, UnsupportedBucket>,
) -> LotSweepStats {
    let mut stats = LotSweepStats {
        lot_name: format!("{label} ({rel_house})"),
        ..LotSweepStats::default()
    };
    let house = root.join(rel_house);
    let mut lot = Lot::load(root, &house, &[3; 32], Frames::default())
        .unwrap_or_else(|e| panic!("failed to load {rel_house}: {e}"));

    // Warm up a few ticks so destination lots spawn arriving NPCs / controllers.
    for tick in 1..=20 {
        if find_actor(lot.world()).is_some() && lot.ticks() >= 5 {
            break;
        }
        match lot.step_play() {
            Ok(update) => {
                for err in &update.errors {
                    stats
                        .warmup_errors
                        .push(format!("tick {tick} (recovered): {}", err.fault));
                }
            }
            Err(StepError::Asking { .. }) => {
                if let Some(asking) = lot.asking() {
                    let ans = asking.shown.answers[0];
                    let _ = lot.answer(ans);
                } else if let Some(sp) = lot.asking_special_panel() {
                    let resp = match sp.shown.request {
                        SpecialPanelRequest::DowntownLotPicker => {
                            SpecialPanelResponse::Lot(Some(21))
                        }
                        SpecialPanelRequest::VacationLotPicker { .. } => {
                            SpecialPanelResponse::Lot(Some(40))
                        }
                        SpecialPanelRequest::OldTownLotPicker { .. } => {
                            SpecialPanelResponse::Lot(Some(58))
                        }
                        SpecialPanelRequest::StudioTownLotPicker { .. } => {
                            SpecialPanelResponse::Lot(Some(81))
                        }
                        SpecialPanelRequest::MagicTownLotPicker { .. } => {
                            SpecialPanelResponse::Lot(Some(93))
                        }
                        SpecialPanelRequest::CallNeighbor { .. } => {
                            SpecialPanelResponse::Neighbor(Some(1))
                        }
                        SpecialPanelRequest::WardrobeShop { .. }
                        | SpecialPanelRequest::PetPicker { .. }
                        | SpecialPanelRequest::CharmTransform { .. } => {
                            SpecialPanelResponse::Selection {
                                accepted: true,
                                index: 0,
                            }
                        }
                        SpecialPanelRequest::SpellBook { .. }
                        | SpecialPanelRequest::CookBook { .. } => SpecialPanelResponse::Dismissed,
                    };
                    let _ = lot.answer_special_panel(resp);
                }
            }
            Err(err) => {
                stats.warmup_errors.push(format!("tick {tick}: {err}"));
                break;
            }
        }
    }
    let Some(actor) = find_actor(lot.world()) else {
        let reason = if stats.warmup_errors.is_empty() {
            "no actor found after warm-up".to_owned()
        } else {
            format!(
                "no actor found after warm-up ({})",
                stats.warmup_errors.join("; ")
            )
        };
        eprintln!("{}: {reason}; skipping", stats.lot_name);
        stats.skipped_reason = Some(reason);
        return stats;
    };
    if select_actor
        && selected(lot.world()).is_none()
        && let Err(err) = lot.select(actor)
    {
        stats
            .warmup_errors
            .push(format!("select actor {actor} failed: {err}"));
    }
    let tuning = lot
        .program()
        .motive_tuning()
        .expect("motive tuning must load");
    let base_world = lot.world().clone();
    let base_rotation = lot.view_rotation();
    let base_centre = lot.view_centre();
    let family_zero_members: Vec<u32> = if lot.world().globals.get(10).copied().unwrap_or(0) != 0
        && lot.world().globals.get(9).copied().unwrap_or(0) == 0
    {
        base_world
            .families
            .iter()
            .find(|f| f.id == 0)
            .map(|f| f.members.clone())
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    let active_family = (lot.world().globals.get(10).copied().unwrap_or(0) != 0
        && lot.world().globals.get(9).copied().unwrap_or(0) == 0)
        .then_some(family_zero_members.as_slice());

    // Deduplicate multi-tile objects by lead tile so we don't repeat identical menus.
    let mut seen_leads = BTreeSet::new();
    let object_ids: Vec<u16> = base_world
        .objects
        .iter()
        .filter_map(|(&id, obj)| {
            let lead = obj
                .multipart
                .as_ref()
                .map_or(id, |m| if m.parent <= 0 { id } else { m.parent as u16 });
            seen_leads.insert(lead).then_some(lead)
        })
        .collect();
    stats.objects_scanned = object_ids.len();

    let mut record_unsupported = |key: String, sample: String, lot_name: &str| {
        let entry = buckets.entry(key).or_default();
        entry.count += 1;
        entry.lots.insert(lot_name.to_owned());
        if entry.samples.len() < 5 {
            entry.samples.push(sample);
        }
    };

    // Deduplicate identical `(object GUID, menu table_index, action tree, join)`
    // entries per lot while still probing every lead object's `pie_menu` so
    // state-dependent menus are never missed and duplicate chairs/shrubs/lamps
    // don't multiply runtime 10x.
    let mut seen_interactions = BTreeSet::new();
    let mut worklist: Vec<(String, World, sims_vm::orders::MenuEntry)> = Vec::new();
    for obj_id in object_ids {
        let obj_guid = base_world.objects.get(&obj_id).map_or(0, |o| o.guid);
        let obj_name = object_label(&lot, obj_id);
        let mut menu_world = base_world.clone();
        let sounds = RefCell::new(Vec::new());
        let mut menu_interp = Interpreter::new(lot.program(), lot.catalog(), Stream::Global);
        menu_interp.sounds = Some(&sounds);
        menu_interp.view_rotation = base_rotation;
        menu_interp.view_centre = base_centre;
        menu_interp.active_family = active_family;
        let entries = match menu_interp.pie_menu(&mut menu_world, actor, obj_id) {
            Ok(entries) => entries,
            Err(fault) => {
                if let Trap::Unsupported(msg) = &fault.trap {
                    stats.pie_menu_unsupported += 1;
                    record_unsupported(
                        format!("[pie_menu] {}", normalize_unsupported(msg)),
                        format!("{label} | {obj_name} | pie_menu | {fault}"),
                        label,
                    );
                }
                continue;
            }
        };
        for entry in entries {
            let key = (
                obj_guid,
                entry.action.table_index,
                entry.action.action,
                entry.action.join,
            );
            if seen_interactions.insert(key) {
                worklist.push((obj_name.clone(), menu_world.clone(), entry));
            }
        }
    }

    stats.entries_tested = worklist.len();
    let workers = std::thread::available_parallelism()
        .map_or(4, |n| n.get().min(8))
        .max(1);
    let chunk_size = worklist.len().div_ceil(workers).max(1);

    #[derive(Default)]
    struct ChunkResult {
        clean: usize,
        native_faults: usize,
        invalid_faults: usize,
        unsupported_hits: usize,
        special_panels_triggered: BTreeMap<u8, usize>,
        unsupported_events: Vec<(String, String)>,
    }

    let program = lot.program();
    let catalog = lot.catalog();
    let tuning_ref = &tuning;
    let chunk_results: Vec<ChunkResult> = std::thread::scope(|scope| {
        let mut handles = Vec::new();
        for chunk in worklist.chunks(chunk_size) {
            handles.push(scope.spawn(move || {
                let mut res = ChunkResult::default();
                for (obj_name, menu_world, entry) in chunk {
                    let entry_label = menu_entry_label(entry);
                    let mut world = menu_world.clone();
                    let special_panels = RefCell::new(SpecialPanels::default());
                    let dialog_text = RefCell::new(String::new());
                    let character_names = RefCell::new(BTreeMap::new());
                    let mut interp = Interpreter::new(program, catalog, Stream::Global);
                    interp.view_rotation = base_rotation;
                    interp.view_centre = base_centre;
                    interp.active_family = active_family;
                    interp.special_panels = Some(&special_panels);
                    interp.dialog_text = Some(&dialog_text);
                    interp.character_names = Some(&character_names);

                    if let Err(fault) = interp.pick(&mut world, actor, entry.action.clone()) {
                        match &fault.trap {
                            Trap::Unsupported(msg) => {
                                res.unsupported_hits += 1;
                                res.unsupported_events.push((
                                    format!("[pick] {}", normalize_unsupported(msg)),
                                    format!("{label} | {obj_name} | {entry_label} | {fault}"),
                                ));
                            }
                            Trap::Native { .. } => res.native_faults += 1,
                            _ => res.invalid_faults += 1,
                        }
                        continue;
                    }

                    let mut player = SweepPlayer::new(&world);
                    let mut play = Play::default();
                    let mut hit_unsupported = false;
                    let mut hit_native = false;
                    let mut hit_invalid = false;
                    for tick in 1..=ticks_per_interaction {
                        let (update, tick_res) = interp.lot_update_kept(
                            &mut world,
                            tuning_ref,
                            &mut player,
                            Some(&mut play),
                        );
                        for err in &update.errors {
                            match &err.fault.trap {
                                Trap::Unsupported(msg) => {
                                    hit_unsupported = true;
                                    res.unsupported_hits += 1;
                                    res.unsupported_events.push((
                                        normalize_unsupported(msg),
                                        format!(
                                            "{label} | {obj_name} | {entry_label} | tick {tick} | {}",
                                            err.fault
                                        ),
                                    ));
                                    break;
                                }
                                Trap::Native { .. } => hit_native = true,
                                _ => hit_invalid = true,
                            }
                        }
                        if hit_unsupported {
                            break;
                        }
                        match tick_res {
                            Ok(_) => {}
                            Err(TickError::Fault(fault)) => {
                                match &fault.trap {
                                    Trap::Unsupported(msg) => {
                                        hit_unsupported = true;
                                        res.unsupported_hits += 1;
                                        res.unsupported_events.push((
                                            normalize_unsupported(msg),
                                            format!(
                                                "{label} | {obj_name} | {entry_label} | tick {tick} | {fault}"
                                            ),
                                        ));
                                    }
                                    Trap::Native { .. } => hit_native = true,
                                    _ => hit_invalid = true,
                                }
                                break;
                            }
                            Err(TickError::Unsupported(msg)) => {
                                hit_unsupported = true;
                                res.unsupported_hits += 1;
                                res.unsupported_events.push((
                                    format!("[tick] {}", normalize_unsupported(&msg)),
                                    format!(
                                        "{label} | {obj_name} | {entry_label} | tick {tick} | {msg}"
                                    ),
                                ));
                                break;
                            }
                            Err(
                                TickError::Unanswered { .. }
                                | TickError::UnansweredSpecialPanel { .. },
                            ) => {
                                hit_invalid = true;
                                break;
                            }
                        }
                    }
                    for panel in &player.panels_seen {
                        *res.special_panels_triggered
                            .entry(panel.kind())
                            .or_default() += 1;
                    }
                    if !hit_unsupported && !hit_invalid {
                        res.clean += 1;
                        if hit_native {
                            res.native_faults += 1;
                        }
                    } else if hit_invalid && !hit_unsupported {
                        res.invalid_faults += 1;
                    }
                }
                res
            }));
        }
        handles
            .into_iter()
            .map(|h| h.join().expect("sweep worker thread panicked"))
            .collect()
    });

    for chunk in chunk_results {
        stats.clean += chunk.clean;
        stats.native_faults += chunk.native_faults;
        stats.invalid_faults += chunk.invalid_faults;
        stats.unsupported_hits += chunk.unsupported_hits;
        for (kind, count) in chunk.special_panels_triggered {
            *stats.special_panels_triggered.entry(kind).or_default() += count;
        }
        for (key, sample) in chunk.unsupported_events {
            record_unsupported(key, sample, label);
        }
    }

    stats
}

#[test]
fn goth_magic_hole_opens_kind13_special_panel_cleanly() {
    let Some(root) = sims_install() else {
        return;
    };
    let house = root.join("UserData/Houses/House05.iff");
    let mut lot =
        Lot::load(&root, &house, &[3], Frames::default()).expect("Goth House05 must boot");
    lot.wait_for_answers();

    // Advance Goth until Mystery Man delivers the Magic Starter Kit (0x197CDB3C)
    // or a Magic Hole (0x2BAA0029) appears on the lot.
    const MAGIC_STARTER_KIT_GUID: u32 = 0x197c_db3c;
    let mut starter_kit_id = None;
    let mut magic_hole_id = None;
    for tick in 1..=2500 {
        match lot.step_play() {
            Ok(_) => {}
            Err(StepError::Asking { .. }) => {
                if let Some(asking) = lot.asking() {
                    let ans = asking.shown.answers[0];
                    lot.answer(ans).unwrap();
                    lot.step_play().unwrap();
                } else {
                    panic!("unexpected special panel during Goth warm-up at tick {tick}");
                }
            }
            Err(e) => panic!("unexpected step error during Goth warm-up at tick {tick}: {e}"),
        }
        magic_hole_id =
            lot.world().objects.iter().find_map(|(&id, obj)| {
                (obj.guid == sims_vm::travel::MAGIC_PORTAL_GUID).then_some(id)
            });
        starter_kit_id = lot
            .world()
            .objects
            .iter()
            .find_map(|(&id, obj)| (obj.guid == MAGIC_STARTER_KIT_GUID).then_some(id));
        if magic_hole_id.is_some() || starter_kit_id.is_some() {
            break;
        }
    }

    let actor = selected(lot.world()).expect("Goth must have a selected Sim");
    if magic_hole_id.is_none()
        && let Some(kit_id) = starter_kit_id
    {
        // Step a few ticks until the Mystery Man finishes placing the Starter Kit
        // so "Open" (0x1001) becomes offered on its pie menu.
        let mut opened = false;
        for _ in 0..400 {
            if let Ok(menu) = lot.pie_menu(actor, kit_id)
                && let Some(open_entry) = menu.entries.iter().find(|e| e.action.action == 0x1001)
            {
                let action = open_entry.action.clone();
                lot.pick(actor, action)
                    .expect("picking Open on Magic Starter Kit must succeed");
                opened = true;
                break;
            }
            match lot.step_play() {
                Ok(_) => {}
                Err(StepError::Asking { .. }) => {
                    if let Some(asking) = lot.asking() {
                        let ans = asking.shown.answers[0];
                        lot.answer(ans).unwrap();
                        lot.step_play().unwrap();
                    }
                }
                Err(e) => panic!("step error waiting for Starter Kit Open: {e}"),
            }
        }
        assert!(opened, "Magic Starter Kit must offer Open (0x1001) on Goth");

        // Step until opening the Starter Kit spawns the Magic Hole (ClownPortal).
        for tick in 1..=600 {
            match lot.step_play() {
                Ok(_) => {}
                Err(StepError::Asking { .. }) => {
                    if let Some(asking) = lot.asking() {
                        let ans = asking.shown.answers[0];
                        lot.answer(ans).unwrap();
                        lot.step_play().unwrap();
                    }
                }
                Err(e) => panic!("Magic Starter Kit Open failed at tick {tick}: {e}"),
            }
            magic_hole_id = lot.world().objects.iter().find_map(|(&id, obj)| {
                (obj.guid == sims_vm::travel::MAGIC_PORTAL_GUID).then_some(id)
            });
            if magic_hole_id.is_some() {
                break;
            }
        }
    }

    let magic_hole_id =
        magic_hole_id.expect("Goth House05 must spawn the Magic Hole (ClownPortal)");

    // Let the Magic Starter Kit "Open" animation and ClownPortal placement finish
    // so the Magic Hole's routing slots are clear before picking Jump In.
    for _ in 0..200 {
        match lot.step_play() {
            Ok(_) => {}
            Err(StepError::Asking { .. }) => {
                if let Some(asking) = lot.asking() {
                    let ans = asking.shown.answers[0];
                    lot.answer(ans).unwrap();
                    lot.step_play().unwrap();
                }
            }
            Err(e) => panic!("step error settling Magic Hole: {e}"),
        }
    }

    // Pick a free Goth household Sim (e.g. Bella) who isn't boxed in by the
    // Starter Kit items, and place the Magic Hole in a nearby clear tile.
    let residents: Vec<u16> = lot
        .world()
        .people
        .iter()
        .copied()
        .filter(|id| {
            lot.world()
                .objects
                .get(id)
                .and_then(|o| o.person.as_ref())
                .is_some_and(|p| p.data.get(32).copied().unwrap_or(0) == 0)
        })
        .collect();
    let jumper = residents
        .iter()
        .copied()
        .find(|&id| id != actor)
        .unwrap_or(actor);
    let jumper_pos = lot.world().objects.get(&jumper).unwrap().position;
    let jx = jumper_pos[0] / 16;
    let jy = jumper_pos[1] / 16;

    lot.set_mode(sims_vm::PanelMode::Buy)
        .expect("enter Buy mode");
    let mut cursor = lot
        .buy_pick_up(magic_hole_id)
        .expect("buy_pick_up on Magic Hole")
        .expect("Magic Hole must be pickable");
    let mut placed = false;
    'find_near: for r in 2..=8 {
        for dy in -r..=r {
            for dx in -r..=r {
                let tx = jx + dx;
                let ty = jy + dy;
                if !(2..62).contains(&tx) || !(2..62).contains(&ty) {
                    continue;
                }
                if lot
                    .buy_move(&mut cursor, [tx * 16 + 8, ty * 16 + 8], 1)
                    .unwrap_or(false)
                {
                    lot.buy_commit(&cursor).expect("buy_commit Magic Hole");
                    placed = true;
                    break 'find_near;
                }
            }
        }
    }
    assert!(placed, "must find a nearby tile for Magic Hole");
    lot.set_mode(sims_vm::PanelMode::Live)
        .expect("return to Live mode");

    lot.select(jumper).expect("select jumper");
    let menu = lot
        .pie_menu(jumper, magic_hole_id)
        .expect("Magic Hole pie_menu must succeed");
    let jump_in = menu
        .entries
        .iter()
        .find(|e| e.action.action == 0x1001)
        .or_else(|| menu.entries.first())
        .expect("Magic Hole must offer Jump In (0x1001)")
        .action
        .clone();
    lot.pick(jumper, jump_in)
        .expect("picking Jump In must succeed");

    // Step until the Magic Hole opens Dialog kind 13 (MagicTownLotPicker).
    let mut saw_magic_picker = false;
    for tick in 1..=800 {
        match lot.step_play() {
            Ok(u) => {
                for err in &u.errors {
                    assert!(
                        !matches!(err.fault.trap, Trap::Unsupported(_)),
                        "Magic Hole must not hit Trap::Unsupported at tick {tick}: {}",
                        err.fault
                    );
                }
            }
            Err(StepError::Asking { tick: asking_tick }) => {
                if let Some(asking) = lot.asking() {
                    let ans = asking.shown.answers[0];
                    lot.answer(ans).unwrap();
                    lot.step_play().unwrap();
                } else if let Some(sp) = lot.asking_special_panel() {
                    let expected_shown = sp.shown.clone();
                    assert_eq!(
                        expected_shown.request,
                        SpecialPanelRequest::MagicTownLotPicker { via_blimp: false },
                        "Magic Hole must trigger MagicTownLotPicker (kind 13, via_blimp: false)"
                    );
                    saw_magic_picker = true;

                    // Verify save/restore while waiting on a special panel!
                    let saved = lot.save().expect("save while asking special panel");
                    let bytes = saved.encode();
                    let decoded = sims_vm::boot::save::Saved::decode(&bytes)
                        .expect("decode v9 save while asking special panel");
                    let mut restored = Lot::load(&root, &house, &[], Frames::default()).unwrap();
                    restored.wait_for_answers();
                    restored.restore(decoded).unwrap();
                    assert_eq!(
                        restored.step_play().unwrap_err(),
                        StepError::Asking { tick: asking_tick }
                    );
                    assert_eq!(
                        restored.asking_special_panel().unwrap().shown,
                        expected_shown
                    );

                    // Pick Magic Town community lot 93 on both live and restored lots
                    // and verify both consume `magic_lot = 93` identically.
                    lot.answer_special_panel(SpecialPanelResponse::Lot(Some(93)))
                        .unwrap();
                    let replay = lot.step_play().expect("replaying MagicTownLotPicker tick");
                    assert_eq!(
                        replay.special_panels_answered,
                        vec![(
                            SpecialPanelRequest::MagicTownLotPicker { via_blimp: false },
                            SpecialPanelResponse::Lot(Some(93))
                        )]
                    );
                    assert_eq!(lot.special_panels().magic_lot, 93);

                    restored
                        .answer_special_panel(SpecialPanelResponse::Lot(Some(93)))
                        .unwrap();
                    let restored_replay = restored.step_play().unwrap();
                    assert_eq!(
                        restored_replay.special_panels_answered,
                        replay.special_panels_answered
                    );
                    assert_eq!(restored.world().hash(), lot.world().hash());
                    break;
                } else {
                    panic!(
                        "StepError::Asking at tick {tick} with neither asking nor asking_special_panel"
                    );
                }
            }
            Err(e) => panic!("Magic Hole halted unexpectedly at tick {tick}: {e}"),
        }
    }
    assert!(
        saw_magic_picker,
        "Goth Magic Hole 'Jump In' must reach Dialog kind 13 (MagicTownLotPicker)"
    );
}

#[test]
fn core_interactive_objects_on_goth_and_newbies_avoid_unsupported() {
    let Some(root) = sims_install() else {
        return;
    };

    let mut objects_tested = 0usize;
    let mut entries_tested = 0usize;
    let mut special_panels_seen = BTreeSet::new();

    for rel_house in ["UserData/Houses/House05.iff", "UserData/Houses/House07.iff"] {
        let house = root.join(rel_house);
        let mut lot = Lot::load(&root, &house, &[3; 32], Frames::default())
            .unwrap_or_else(|e| panic!("failed to load {rel_house}: {e}"));
        for _ in 0..5 {
            let _ = lot.step_play();
        }
        let actor = find_actor(lot.world()).expect("residential lot must have an actor");
        let tuning = lot.program().motive_tuning().expect("motive tuning");
        let base_world = lot.world().clone();
        let base_rotation = lot.view_rotation();
        let base_centre = lot.view_centre();

        let mut seen_leads = BTreeSet::new();
        for (&id, obj) in &base_world.objects {
            if obj.position == [0, 0, 0] {
                continue;
            }
            let lead = obj
                .multipart
                .as_ref()
                .map_or(id, |m| if m.parent <= 0 { id } else { m.parent as u16 });
            if !seen_leads.insert(lead) || !is_catalog_object(&lot, lead) {
                continue;
            }
            let label = object_label(&lot, lead);

            let mut menu_world = base_world.clone();
            let sounds = RefCell::new(Vec::new());
            let mut menu_interp = Interpreter::new(lot.program(), lot.catalog(), Stream::Global);
            menu_interp.sounds = Some(&sounds);
            menu_interp.view_rotation = base_rotation;
            menu_interp.view_centre = base_centre;
            let entries = menu_interp
                .pie_menu(&mut menu_world, actor, lead)
                .unwrap_or_else(|f| panic!("{rel_house} {label} pie_menu failed: {f}"));
            if entries.is_empty() {
                continue;
            }
            objects_tested += 1;

            for entry in entries {
                entries_tested += 1;
                let entry_label = menu_entry_label(&entry);
                let mut world = menu_world.clone();
                let special_panels = RefCell::new(SpecialPanels::default());
                let dialog_text = RefCell::new(String::new());
                let character_names = RefCell::new(BTreeMap::new());
                let mut interp = Interpreter::new(lot.program(), lot.catalog(), Stream::Global);
                interp.view_rotation = base_rotation;
                interp.view_centre = base_centre;
                interp.special_panels = Some(&special_panels);
                interp.dialog_text = Some(&dialog_text);
                interp.character_names = Some(&character_names);

                interp
                    .pick(&mut world, actor, entry.action)
                    .unwrap_or_else(|f| {
                        panic!("{rel_house} {label} pick '{entry_label}' failed: {f}")
                    });

                let mut player = SweepPlayer::new(&world);
                let mut play = Play::default();
                for tick in 1..=200 {
                    let (u, res) =
                        interp.lot_update_kept(&mut world, &tuning, &mut player, Some(&mut play));
                    for err in &u.errors {
                        assert!(
                            !matches!(err.fault.trap, Trap::Unsupported(_)),
                            "{rel_house} {label} '{entry_label}' hit Trap::Unsupported at tick {tick}: {}",
                            err.fault
                        );
                    }
                    if let Err(err) = res {
                        panic!("{rel_house} {label} '{entry_label}' failed at tick {tick}: {err}");
                    }
                }
                for panel in &player.panels_seen {
                    special_panels_seen.insert(panel.kind());
                }
            }
        }
    }

    assert!(
        objects_tested >= 30 && entries_tested >= 100,
        "expected broad coverage on Goth + Newbies; tested {objects_tested} objects, {entries_tested} entries"
    );
    eprintln!(
        "Verified {objects_tested} clickable catalog objects ({entries_tested} interactions) on Goth + Newbies with 0 Trap::Unsupported (special panels triggered: {special_panels_seen:?})"
    );
}

#[test]
#[ignore = "full 9-lot Complete Collection sweep; run explicitly with --ignored --nocapture"]
fn complete_collection_interactive_object_sweep() {
    let Some(root) = sims_install() else {
        eprintln!("SIMS_INSTALL not set; skipping Complete Collection interactive object sweep");
        return;
    };

    // Mirror `tests/destination_lots.rs`: Vacation (`House40`) requires `global[3] != 0`
    // to avoid the Maxis uninitialized `local 6` bug in `NPC_Vacation_Controller.iff`,
    // whereas other destination lots run without selecting an NPC/visitor.
    let lots = [
        ("Goth (House05)", "UserData/Houses/House05.iff", false),
        ("Newbies (House07)", "UserData/Houses/House07.iff", false),
        ("Charming (House69)", "UserData/Houses/House69.iff", false),
        ("Downtown (House21)", "UserData/Houses/House21.iff", false),
        ("Vacation (House40)", "UserData/Houses/House40.iff", true),
        ("Old Town (House58)", "UserData/Houses/House58.iff", false),
        (
            "Studio Town (House81)",
            "UserData/Houses/House81.iff",
            false,
        ),
        (
            "Magic Town Res (House90)",
            "UserData/Houses/House90.iff",
            false,
        ),
        (
            "Magic Town Comm (House93)",
            "UserData/Houses/House93.iff",
            false,
        ),
    ];

    let mut buckets: BTreeMap<String, UnsupportedBucket> = BTreeMap::new();
    let mut all_stats = Vec::new();
    for (label, rel, select_actor) in lots {
        let stats = sweep_one_lot(&root, label, rel, select_actor, 200, &mut buckets);
        eprintln!(
            "{}: objects={} entries={} clean={} native={} invalid={} unsupported={} pie_unsup={} warmup_errs={} skipped={:?} panels={:?}",
            stats.lot_name,
            stats.objects_scanned,
            stats.entries_tested,
            stats.clean,
            stats.native_faults,
            stats.invalid_faults,
            stats.unsupported_hits,
            stats.pie_menu_unsupported,
            stats.warmup_errors.len(),
            stats.skipped_reason,
            stats.special_panels_triggered,
        );
        all_stats.push(stats);
    }

    let mut ranked: Vec<(&String, &UnsupportedBucket)> = buckets.iter().collect();
    ranked.sort_by(|a, b| b.1.count.cmp(&a.1.count).then_with(|| a.0.cmp(b.0)));

    let mut report = String::new();
    let _ = writeln!(
        report,
        "# Complete Collection Interactive Object Sweep Report\n"
    );
    let _ = writeln!(
        report,
        "Swept 9 representative lots (base + expansions), 200 strict ticks per pie-menu entry.\n"
    );
    let _ = writeln!(report, "## Per-Lot Summary\n");
    let _ = writeln!(
        report,
        "| Lot | Status | Objects | Entries | Clean | Native Fault | Invalid/Other | Unsupported | Pie-Menu Unsup | Special Panels |"
    );
    let _ = writeln!(report, "|---|---|---:|---:|---:|---:|---:|---:|---:|---|");
    let (
        mut tot_obj,
        mut tot_ent,
        mut tot_clean,
        mut tot_nat,
        mut tot_inv,
        mut tot_unsup,
        mut tot_pie,
    ) = (0, 0, 0, 0, 0, 0, 0);
    for s in &all_stats {
        tot_obj += s.objects_scanned;
        tot_ent += s.entries_tested;
        tot_clean += s.clean;
        tot_nat += s.native_faults;
        tot_inv += s.invalid_faults;
        tot_unsup += s.unsupported_hits;
        tot_pie += s.pie_menu_unsupported;
        let status = if let Some(reason) = &s.skipped_reason {
            format!("SKIPPED ({reason})")
        } else if !s.warmup_errors.is_empty() {
            format!("warmup warn ({})", s.warmup_errors.join("; "))
        } else {
            "ok".to_owned()
        };
        let panels = if s.special_panels_triggered.is_empty() {
            "-".to_owned()
        } else {
            s.special_panels_triggered
                .iter()
                .map(|(k, c)| format!("kind {k} x{c}"))
                .collect::<Vec<_>>()
                .join(", ")
        };
        let _ = writeln!(
            report,
            "| {} | {} | {} | {} | {} | {} | {} | {} | {} | {} |",
            s.lot_name,
            status,
            s.objects_scanned,
            s.entries_tested,
            s.clean,
            s.native_faults,
            s.invalid_faults,
            s.unsupported_hits,
            s.pie_menu_unsupported,
            panels
        );
    }
    let _ = writeln!(
        report,
        "| **TOTAL** | | **{tot_obj}** | **{tot_ent}** | **{tot_clean}** | **{tot_nat}** | **{tot_inv}** | **{tot_unsup}** | **{tot_pie}** | |"
    );
    let skipped: Vec<&LotSweepStats> = all_stats
        .iter()
        .filter(|s| s.skipped_reason.is_some() || !s.warmup_errors.is_empty())
        .collect();
    if !skipped.is_empty() {
        let _ = writeln!(report, "\n## Warm-Up Warnings & Skipped Lots\n");
        for s in skipped {
            let _ = writeln!(
                report,
                "- **{}**: skipped={:?}, warmup_errors={:?}",
                s.lot_name, s.skipped_reason, s.warmup_errors
            );
        }
    }

    let _ = writeln!(
        report,
        "\n## Ranked Distinct `Trap::Unsupported` Hits ({} distinct)\n",
        ranked.len()
    );
    for (rank, (msg, bucket)) in ranked.iter().enumerate() {
        let lots_list: Vec<&str> = bucket.lots.iter().map(String::as_str).collect();
        let _ = writeln!(
            report,
            "### {}. ({} hits; lots: {})\n`{}`\n",
            rank + 1,
            bucket.count,
            lots_list.join(", "),
            msg
        );
        for sample in &bucket.samples {
            let _ = writeln!(report, "- {sample}");
        }
        let _ = writeln!(report);
    }

    if let Ok(path) = std::env::var("SIMS_SWEEP_REPORT") {
        std::fs::write(&path, &report).expect("failed to write SIMS_SWEEP_REPORT");
        eprintln!("Wrote sweep report to {path}");
    }
    eprintln!(
        "\nTOTAL: objects={tot_obj} entries={tot_ent} clean={tot_clean} unsupported={tot_unsup} pie_unsup={tot_pie} distinct_unsupported={}",
        ranked.len()
    );
}
