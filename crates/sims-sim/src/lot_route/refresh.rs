//! House refresh 444805/45eddc/46387c and the 45ad63 resume prefix.
//! Invalidation preserves actions, paths, candidates and the selected retry.
use crate::Simulation;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resume {
    Continue,
    ForgetPortal,
    StopWalking,
}

/// 45ad63 consumes context+78 only while its retained target pointer exists.
/// Stopping states9/10/11/15 keep their already selected outcome.
pub fn resume(valid: &mut bool, target_present: bool, state: u32) -> Resume {
    if *valid || !target_present {
        return Resume::Continue;
    }
    *valid = true;
    match state {
        2 => Resume::ForgetPortal,
        3 => Resume::StopWalking,
        _ => Resume::Continue,
    }
}

impl Simulation {
    /// 444805 clears every registered person's retained contexts, then46387c
    /// fills allocated portal-cost cells with10001. Implicit missing cells
    /// already read as10001; existing cells/owners must remain allocated.
    pub fn invalidate_original_route_caches(&mut self) -> Result<(), String> {
        if self.native_lot.is_none() {
            return Err("route invalidation requires an original lot".into());
        }
        let people = &self
            .vm
            .world
            .person
            .as_ref()
            .ok_or("route registry absent")?
            .person_order;
        for id in people {
            if self
                .vm
                .world
                .objects
                .get(id)
                .is_none_or(|o| o.person.is_none())
            {
                return Err("registered route person absent".into());
            }
        }
        for id in people {
            for c in self.route_control.values_mut().filter(|c| c.caller == *id) {
                if let Some(original) = &mut c.original {
                    original.cache_valid = false;
                }
            }
        }
        for cache in self.portal_cache.values_mut() {
            cache.invalidate();
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn valid_defaults_preserve_old_checkpoints_and_stale_flag_survives_save() {
        let old = serde_json::json!({"ignored":7,"crossing":false,"blocked_object":0,"seated":null,"standing":null});
        let mut state: crate::lot_route::portal::Control =
            serde_json::from_value(old.clone()).unwrap();
        assert!(state.cache_valid);
        assert_eq!(serde_json::to_value(&state).unwrap(), old);
        state.cache_valid = false;
        let saved = serde_json::to_value(&state).unwrap();
        assert_eq!(saved["cache_valid"], false);
        assert_eq!(
            serde_json::from_value::<crate::lot_route::portal::Control>(saved).unwrap(),
            state
        );
    }
    #[cfg(feature = "private-fixtures")]
    #[test]
    fn invalidated_resume_matches_original_route_prefix() {
        let corpus: serde_json::Value = serde_json::from_str(include_str!(
            "../../tests/fixtures/native-route-refresh-resume.json"
        ))
        .unwrap();
        assert_eq!(corpus["cases"].as_array().unwrap().len(), 144);
        for row in corpus["cases"].as_array().unwrap() {
            let c = &row["input"];
            let n = &row["original"];
            let state = c["state"].as_u64().unwrap() as u32;
            let mut valid = c["valid"].as_u64().unwrap() != 0;
            let action = resume(&mut valid, true, state);
            assert_eq!(valid, n["valid"].as_u64().unwrap() != 0, "{c}");
            assert_eq!(
                action == Resume::ForgetPortal,
                c["previous"] == true && n["previous"] == false || c["valid"] == 0 && state == 2,
                "{c}"
            );
            let calls = n["calls"].as_array().unwrap();
            if state == 3 {
                assert_eq!(action == Resume::StopWalking, calls[0] == "stop", "{c}");
                assert_eq!(action == Resume::Continue, calls[0] == "poll", "{c}");
            }
            if [9, 10, 11, 15].contains(&state) {
                assert_eq!(action, Resume::Continue);
            }
        }
        let mut valid = false;
        assert_eq!(resume(&mut valid, false, 3), Resume::Continue);
        assert!(!valid);
    }
}
