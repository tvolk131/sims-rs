//! Replacement configuration at the already-loaded original caller. These
//! checks do not claim the original has matching Rust configuration flags.
use super::{Result, Saved, Value, fact, json, same};
pub fn evaluate(saved: &Saved) -> Result<Value> {
    let super::super::Options {
        initial_process_profile,
        application_context,
        scheduler,
        native_load_completion,
        native_postload,
        routing,
        crt_seed,
        defer_indicators,
    } = &saved.options;
    let mut rows = vec![];
    fact(
        "initial_process_profile",
        "no_replayed_process_constructor",
        initial_process_profile.is_none(),
        &mut rows,
    );
    fact(
        "application_context",
        "no_replayed_house_load",
        application_context.is_none(),
        &mut rows,
    );
    same(
        "scheduler/replacement_implementation",
        &super::super::RegistryScheduler,
        scheduler,
        &mut rows,
    )?;
    fact(
        "native_load_completion",
        "no_new_load_caller",
        !*native_load_completion,
        &mut rows,
    );
    fact(
        "native_postload",
        "no_new_postload_caller",
        !*native_postload,
        &mut rows,
    );
    same(
        "routing/replacement_implementation",
        &sims_sim::lot_route::path::Policy::Strict,
        routing,
        &mut rows,
    )?;
    // This exact native interval has no RNG draws. The independently compared
    // CRT stream therefore still equals the source seed retained by Options.
    same(
        "crt_seed/source_stream_alias_for_this_no_draw_interval",
        &saved.simulation.state.vm.crt_rng_state,
        &Some(*crt_seed),
        &mut rows,
    )?;
    fact(
        "defer_indicators",
        "retained_loaded_indicator_owners",
        !*defer_indicators,
        &mut rows,
    );
    anyhow::ensure!(rows.len() == 8, "Options field coverage inventory changed");
    let differences = rows
        .iter()
        .filter(|r| r.get("matched") == Some(&Value::Bool(false)))
        .count();
    Ok(
        json!({"schema":1,"comparisons":rows,"difference_count":differences,
        "native_parity_ticks":0,"complete_native_Session_admitted":false,
        "scope":"All8 replacement Options fields; actual native state remains in the separately compared retained owners."}),
    )
}
