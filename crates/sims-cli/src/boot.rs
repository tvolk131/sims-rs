//! `sims boot`: run the lot load for a house, then as many ticks as asked
//! for, and print the hash of the state it reaches. A load that completes
//! ends at the state the original's first tick starts from. A tick that
//! reaches something not ported stops the boot, which says at which tick.
//! The work is `sims_vm::boot`; this prints it.

use sims_sim::Snapshots;
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
    pub snapshot: Option<&'a Path>,
    /// When the house view draws a frame ([`Frames`]).
    pub frames: Frames,
    /// Step in play mode ([`Lot::step_play`]): an object's error resets
    /// the object instead of stopping the boot. Not for comparing with
    /// the original.
    pub play: bool,
}

pub fn run(options: &Options<'_>) -> Result<String, Box<dyn Error>> {
    let mut lot = Lot::load(
        options.root,
        options.house,
        &options.answers,
        options.frames.clone(),
    )
    .map_err(|error| -> Box<dyn Error> { error })?;
    // The ticks: one lot update each, with the house view's frames
    // between them.
    let mut ticked = Vec::new();
    let mut failed = None;
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
                failed = Some(error.to_string());
                break;
            }
        }
    }
    if let Some(path) = options.snapshot
        && failed.is_none()
    {
        Snapshots::open(path)?.save("boot", lot.world())?;
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
        let errors: Vec<_> = ticked.iter().flat_map(|u| &u.errors).collect();
        let frames: Vec<_> = ticked.iter().flat_map(|u| &u.frame_errors).collect();
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
