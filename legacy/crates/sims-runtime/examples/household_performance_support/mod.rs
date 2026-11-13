use serde_json::{Value, json};
use sims_runtime::original_session::Session;
use std::collections::BTreeMap;

pub mod exact_tick;

pub fn calendar(session: &Session) -> Option<i64> {
    session.simulation.household.as_ref().map(|household| {
        let clock = &household.clock;
        i64::from(clock.elapsed_days) * 86400
            + i64::from(clock.hour) * 3600
            + i64::from(clock.minute) * 60
            + i64::from(clock.second)
    })
}

pub fn state_summary(session: &Session) -> Value {
    let mut statuses = BTreeMap::<String, usize>::new();
    let mut faults = BTreeMap::new();
    for (thread, state) in &session.simulation.vm.threads {
        let status = format!("{:?}", state.status);
        *statuses
            .entry(status.split('(').next().unwrap_or(&status).to_string())
            .or_default() += 1;
        if let sims_vm::Status::Faulted(fault) = &state.status {
            faults.insert(thread.to_string(), format!("{fault:?}"));
        }
    }
    json!({"tick":session.simulation.vm.tick,"calendar_seconds":calendar(session),
        "rng":session.simulation.vm.rng_state,"lot_rng":session.simulation.vm.lot_rng_state,
        "crt_rng":session.simulation.vm.crt_rng_state,"global17":session.simulation.vm.world.globals.get(&17),
        "global19":session.simulation.vm.world.globals.get(&19),"characters":session.simulation.characters.len(),
        "objects":session.simulation.vm.world.objects.len(),"thread_statuses":statuses,"faults":faults,
        "modal":session.simulation.vm.world.dialogs.modal})
}

pub fn latency(samples: &[f64]) -> Value {
    if samples.is_empty() {
        return json!({"samples":0});
    }
    let mut sorted = samples.to_vec();
    sorted.sort_by(f64::total_cmp);
    let percentile = |numerator: usize| sorted[(sorted.len() * numerator).div_ceil(100) - 1];
    json!({"samples":sorted.len(),"median":percentile(50),"p95":percentile(95),
        "p99":percentile(99),"max":sorted[sorted.len()-1],
        "over_16_7_ms":samples.iter().filter(|&&sample|sample>1000./60.).count(),
        "over_40_ms":samples.iter().filter(|&&sample|sample>40.).count(),
        "method":"nearest rank; per synchronous step, not frame latency"})
}

#[cfg(test)]
mod tests {
    use super::latency;

    #[test]
    fn nearest_rank_handles_empty_singleton_and_unsorted_samples() {
        assert_eq!(latency(&[]), serde_json::json!({"samples":0}));
        let singleton = latency(&[42.]);
        assert_eq!(singleton["p99"], 42.);
        assert_eq!(singleton["over_40_ms"], 1);
        let samples = latency(&[40., 1., 16., 17., 50.]);
        assert_eq!(samples["median"], 17.);
        assert_eq!(samples["p95"], 50.);
        assert_eq!(samples["over_16_7_ms"], 3);
        assert_eq!(samples["over_40_ms"], 1);
    }
}
