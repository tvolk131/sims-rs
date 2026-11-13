//! Retain46d9d6 at the captured46d345 return, without frame entry or setters.
use super::{clock, completed_controls, completed_lot::Memory, transport};
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use sims_vm::scheduler::{
    clock::{Fields, Timer},
    transport::Timing,
};
pub mod captured;
fn word(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(b[o..o + 4].try_into().unwrap())
}
fn bytes(v: &Value) -> Result<Vec<u8>> {
    super::completed_animations::bytes(v.as_str().context("elapsed source bytes absent")?)
}
fn pointer(v: &Value) -> Result<u32> {
    Ok(v.as_u64()
        .context("elapsed source pointer absent")?
        .try_into()?)
}

/// Actual scalar owners at either completed endpoint. No wrapper is created.
pub fn fields(v: &Value) -> Result<Fields> {
    let controls = completed_controls::import(v)?;
    let m = Memory::new(v)?;
    for (a, n, h) in [
        (
            0x46d9d6,
            0x130,
            "25d16b6553a209d266b1b2ec04068ff56432af7fea2738fd3b3c53d1462eb0ff",
        ),
        (
            0x46dbab,
            0x100,
            "cf33bcd6e734ae8f952d3b1a50e2f3590519226140244725867d3d44cdf08670",
        ),
    ] {
        ensure!(
            sims_fixtures::digest(&m.read(a, n)?) == h,
            "actual elapsed code profile differs"
        );
    }
    ensure!(
        m.read(0x46da8c, 4)? == [0xcc; 4],
        "captured protected poll witness differs"
    );
    let simulator = pointer(&v["simulator"])?;
    let b = m.read(simulator + 0xb8, 0x29)?;
    ensure!(
        b == bytes(&v["elapsed_raw"])?,
        "elapsed scalar readset differs"
    );
    let fields = Fields {
        timing: Timing {
            last: word(&b, 0),
            elapsed: word(&b, 4),
        },
        period: word(&b, 8) as i32,
        timer: Timer {
            elapsed: word(&b, 12),
            interval: word(&b, 16),
            last: word(&b, 20),
            limit: word(&b, 24),
        },
        poll_messages: b[28],
        budget_d8: word(&b, 32) as i32,
        budget_dc: word(&b, 36) as i32,
        publish: b[40],
    };
    ensure!(
        fields.timing == controls.timing,
        "elapsed and transport counter owners differ"
    );
    ensure!(
        fields.poll_messages == 0,
        "captured protected input poll is not an executable supported branch"
    );
    let requests = m.read(simulator + 0x1a4, 8)?;
    ensure!(
        requests == bytes(&v["requests_raw"])?,
        "elapsed request header differs"
    );
    let head = word(&requests, 0);
    let sentinel = m.read(head, 8)?;
    ensure!(
        sentinel == bytes(&v["sentinel_raw"])?
            && word(&requests, 4) == 0
            && word(&sentinel, 0) == head
            && word(&sentinel, 4) == head
            && v["requests"] == json!([]),
        "nonempty or contradictory original elapsed requests require their retained service owner"
    );
    Ok(fields)
}

/// A new Rust adapter around the actual already-returning native child. No
/// accepted Rust request, constructor, timer credit, pending notification or
/// application-state query is reconstructed.
pub fn resume(v: &Value, closure: &Value, tick: u64) -> Result<(transport::State, clock::State)> {
    ensure!(
        tick == 88448 && v["tick"] == tick && closure["tick"] == tick,
        "elapsed resume is not the qualified initial completed boundary"
    );
    ensure!(
        closure["image_sha256"] == v["provenance"]["image_sha256"]
            && closure["observation_sha256"]
                == "63d1eff4057d012969978990c689889561e158f58ccee5e894a536f2c570e00c"
            && closure["source_frame_unwind"]["completed_tick_ebp"] == 8518784u32
            && closure["source_frame_unwind"]["returns"]
                == json!(["46da75", "4d1ab8", "4f7563", "5d5e7c"]),
        "elapsed original caller proof differs"
    );
    let stack = Memory::new(closure)?;
    for (a, expected) in [
        (0x81fc84, 0x46da75),
        (0x81fcb4, 0x4d1ab8),
        (0x81fe08, 0x4f7563),
        (0x81fe18, 0x5d5e7c),
    ] {
        ensure!(
            word(&stack.read(a, 4)?, 0) == expected,
            "elapsed original return storage differs"
        );
    }
    ensure!(
        word(&stack.read(0x81fc50, 4)?, 0) == pointer(&v["simulator"])?,
        "elapsed completed child and simulator differ"
    );
    let fields = fields(v)?;
    let m = Memory::new(v)?;
    for (a, n) in [(0x46d9d6, 182), (0x46da90, 72)] {
        ensure!(
            stack.read(a, n)? == m.read(a, n)?,
            "elapsed scalar and original stack code epochs differ"
        );
    }
    let now = m.read(0x6e54e0, 4)?;
    let stopping = m.read(0x6e54a8, 1)?;
    ensure!(
        now == bytes(&v["now_raw"])? && stopping == bytes(&v["stopping_raw"])?,
        "elapsed application inputs differ"
    );
    let mut transport = completed_controls::import(v)?.into_transport();
    // Transfer the observed counters to the sole enclosing clock owner.
    transport.timing = None;
    let clock = clock::State::from_completed_update(fields, word(&now, 0), stopping[0], tick);
    Ok((transport, clock))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn load(path: &str) -> Result<Value> {
        Ok(serde_json::from_slice(&std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../..")
                .join(path),
        )?)?)
    }
    #[test]
    #[ignore = "private actual elapsed scalar and native stack receipts"]
    fn actual_completed_elapsed_owns_counters_and_rejects_changed_return_or_epoch() -> Result<()> {
        let v =
            load("fixtures/private/goth-elapsed-parent-20261029-first/controls-before-first.json")?;
        let reentry = load(
            "fixtures/private/goth-completed-stack-comparison-20261027-first/reentry-input-third.json",
        )?;
        let closure = &reentry["source_closure"];
        let (transport, clock) = resume(&v, closure, 88448)?;
        assert_eq!(transport.timing, None);
        assert!(clock.pending());
        assert_eq!(
            clock.fields.timing,
            Timing {
                last: 343485,
                elapsed: 93
            }
        );
        assert_eq!(
            clock.fields.timer,
            Timer {
                elapsed: 1218,
                interval: 42,
                last: 343485,
                limit: 30
            }
        );
        assert!(clock.requests.is_empty());
        assert_eq!(clock.now, 343485);
        let after = fields(&load(
            "fixtures/private/goth-elapsed-parent-20261029-first/controls-after-first.json",
        )?)?;
        assert_eq!(after.timing.elapsed, 51);
        assert_eq!(after.timer.elapsed, 1176);
        assert!(resume(&v, closure, 88449).is_err());
        let mut wrong = closure.clone();
        wrong["source_frame_unwind"]["returns"][0] = json!("different");
        assert!(resume(&v, &wrong, 88448).is_err());
        let mut wrong = v.clone();
        wrong["elapsed_raw"] = json!("00".repeat(0x29));
        assert!(fields(&wrong).is_err());
        let mut wrong = v;
        wrong["requests"] = json!([{}]);
        assert!(fields(&wrong).is_err());
        Ok(())
    }
}
