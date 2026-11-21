//! `sims boot`: run the lot load for a house, then as many ticks as asked
//! for, and print the hash of the state it reaches. A load that completes
//! ends at the state the original's first tick starts from. A tick that
//! reaches something not ported stops the boot, which says at which tick.
//! The work is `sims_vm::boot`; this prints it.

use sims_sim::Snapshots;
use sims_vm::SoundEvent;
pub use sims_vm::boot::Frames;
use sims_vm::boot::{Lot, StepError};
use std::error::Error;
use std::fmt::Write as _;
use std::path::Path;

pub struct Options<'a> {
    pub root: &'a Path,
    /// A house file inside a neighborhood's `Houses` directory.
    pub house: &'a Path,
    pub ticks: u32,
    /// The answers to give, in order, to dialogs that offer a choice. A
    /// dialog with one button takes none.
    pub answers: Vec<u8>,
    /// The view-rotated tile pick at the house view's centre
    /// ([`Lot::set_view_centre`]), which a dialog's portrait resolves into
    /// its search start (`0x434891`, `0x43491c`). None: not known.
    pub view_centre: Option<[i8; 2]>,
    pub snapshot: Option<&'a Path>,
    /// When the house view draws a frame ([`Frames`]).
    pub frames: Frames,
    /// Step in play mode ([`Lot::step_play`]): an object's error resets
    /// the object instead of stopping the boot. Not for comparing with
    /// the original.
    pub play: bool,
}

pub fn run(options: &Options<'_>) -> Result<String, Box<dyn Error>> {
    let mut lot = Lot::load_viewed(
        options.root,
        options.house,
        &options.answers,
        options.frames.clone(),
        options.view_centre,
    )
    .map_err(|error| -> Box<dyn Error> { error })?;
    // The ticks: one lot update each, with the house view's frames
    // between them.
    let mut ticked = Vec::new();
    let mut failed = None;
    // In play mode, what a tick that failed did before it failed.
    let mut stopped_update = None;
    if options.ticks > 0 && lot.load_report().finished.is_none() {
        return Err("ticks were asked for, but the load did not complete".into());
    }
    for _ in 0..options.ticks {
        let step = if options.play {
            lot.step_play()
        } else {
            lot.step()
        };
        match step {
            Ok(update) => ticked.push(update),
            // Missing motive constants stop the run before any tick, with
            // no report, as before the library.
            Err(StepError::Tuning(error)) => return Err(error.into()),
            Err(error) => {
                // A frame that fails after its tick still counts the tick.
                if let StepError::Frame {
                    update: Some(update),
                    ..
                } = &error
                {
                    ticked.push((**update).clone());
                }
                if let StepError::Tick {
                    update: Some(update),
                    ..
                } = &error
                {
                    stopped_update = Some((**update).clone());
                }
                failed = Some(error.to_string());
                break;
            }
        }
    }
    if let Some(path) = options.snapshot
        && failed.is_none()
    {
        Snapshots::open(path)?.save_with_floor_rotation(
            "boot",
            lot.world(),
            lot.floor_rotation(),
        )?;
    }
    let world = lot.world();
    let load = lot.load_report();
    let mut out = String::new();
    let _ = writeln!(out, "House:   {}", options.house.display());
    let _ = writeln!(
        out,
        "Saved:   {} objects ({} people), tick {}",
        load.saved_objects, load.saved_people, load.saved_ticks
    );
    let _ = writeln!(
        out,
        "Linked:  {} files; {} of {} object types match their saved signatures",
        load.libraries,
        load.types - load.mismatches.len(),
        load.types
    );
    for line in &load.mismatches {
        let _ = writeln!(out, "{line}");
    }
    let _ = writeln!(
        out,
        "Load:    lot resized to {} tiles, {} controllers created, {} queued for removal",
        world.size, load.created, load.queued,
    );
    let _ = writeln!(
        out,
        "         lot cleared, {} objects removed; {} lot arrays read, {} floor \
         cells renumbered",
        load.removed,
        world.arrays.len(),
        load.renumbered
    );
    let _ = writeln!(
        out,
        "         type table read, {} types changed; lot cleared again, {} objects removed",
        load.changed, load.cleared_again
    );
    let _ = writeln!(
        out,
        "         {} saved objects read; their constructors drew {} C runtime random numbers",
        load.read, load.draws
    );
    let _ = writeln!(
        out,
        "         {} controllers created among them, {} duplicates removed; {} IDs queued \
         for removal",
        load.reconciled_created, load.reconciled_removed, load.queued_again
    );
    let _ = writeln!(
        out,
        "         ground object grid checked, {} cells cleared",
        load.repaired
    );
    let rooms = &world.rooms.rooms;
    let _ = writeln!(
        out,
        "         {} rooms worked out ({} indoors, {} tiles split by a diagonal wall); {} \
         objects changed room, {} told the rooms changed",
        rooms.len(),
        rooms.values().filter(|room| !room.outdoors).count(),
        world.rooms.halves.len(),
        load.moved,
        load.asked
    );
    let _ = writeln!(
        out,
        "         rooms counted, indoor area {} tiles; {} queued removals dropped",
        world.globals[27], load.dropped
    );
    let _ = writeln!(
        out,
        "         family {} selected for house {}, funds {}",
        load.family, load.house, world.budget.funds
    );
    let _ = writeln!(
        out,
        "         {} of {} objects visited, {} of them reset; {} IDs queued for removal",
        load.visited,
        load.to_visit,
        load.reset,
        world.removals.len()
    );
    match &load.stopped {
        Some(fault) => {
            let _ = writeln!(
                out,
                "         stopped in step 13 at the next visit: {fault}"
            );
        }
        None => {
            let (made, sim, counted) = load
                .finished
                .map_or((false, None, 0), |f| (f.indicators_made, f.sim, f.counted));
            let _ = writeln!(
                out,
                "         indicators {}; household selection finished; house announced",
                if made {
                    "created"
                } else {
                    "already on the lot"
                }
            );
            let _ = writeln!(
                out,
                "         {}; live mode; {counted} rooms counted, {} still marked",
                match sim {
                    Some(sim) => format!("Sim {sim} selected"),
                    None => "no Sim selected".to_owned(),
                },
                world.rooms.rooms.values().filter(|room| room.dirty).count()
            );
            let _ = writeln!(
                out,
                "         load complete: this is the state the first tick starts from"
            );
        }
    }
    for (tick, update) in ticked.iter().enumerate() {
        let _ = writeln!(
            out,
            "Tick {}:  {} pass; {} counted down, {} ticked; {} dialogs answered{}; {} removed",
            tick + 1,
            if update.full { "full" } else { "partial" },
            update.counted.len(),
            update.ticked.len(),
            update.answered.len(),
            if update.answered.is_empty() {
                String::new()
            } else {
                format!(
                    " ({})",
                    update
                        .answered
                        .iter()
                        .map(|(object, answer)| format!("{object}: {answer}"))
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            },
            update.removed.len()
        );
    }
    if options.play {
        // The tick that failed counts here, though not as a tick ran.
        let steps = || ticked.iter().chain(&stopped_update);
        let errors: Vec<_> = steps().flat_map(|u| &u.errors).collect();
        let frames: Vec<_> = steps().flat_map(|u| &u.frame_errors).collect();
        let _ = writeln!(
            out,
            "Play:    {} errors caught, {} frames skipped{}",
            errors.len(),
            frames.len(),
            frames
                .first()
                .map(|e| format!(" (the first: {e})"))
                .unwrap_or_default()
        );
        let mut played = std::collections::BTreeMap::<&str, usize>::new();
        let mut stopped = 0;
        for event in steps().flat_map(|u| &u.sounds) {
            match event {
                SoundEvent::Play { name, .. } => *played.entry(name).or_default() += 1,
                SoundEvent::Stop { .. } => stopped += 1,
            }
        }
        let mut most: Vec<_> = played.iter().collect();
        most.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
        let _ = writeln!(
            out,
            "Sounds:  {} played, {} stopped{}",
            played.values().sum::<usize>(),
            stopped,
            if most.is_empty() {
                String::new()
            } else {
                format!(
                    " (most played: {})",
                    most.iter()
                        .take(8)
                        .map(|(name, count)| format!("{name} {count}"))
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            }
        );
        for error in errors {
            let _ = writeln!(
                out,
                "         tick {}, object {}{}: {}{}; {:?}",
                error.tick,
                error.object,
                if error.in_thread {
                    ""
                } else {
                    " (outside its thread)"
                },
                error.fault,
                error
                    .then
                    .as_ref()
                    .map(|then| format!("; then outside its thread: {then}"))
                    .unwrap_or_default(),
                error.recovery
            );
        }
    }
    if let Some(failed) = failed {
        let _ = writeln!(out, "Stopped: {failed}");
        return Err(out.into());
    }
    let _ = writeln!(
        out,
        "Lot:     {} objects, tick {}",
        world.objects.len(),
        world.ticks
    );
    let _ = writeln!(out, "Hash:    {}", world.hash_hex());
    Ok(out)
}
