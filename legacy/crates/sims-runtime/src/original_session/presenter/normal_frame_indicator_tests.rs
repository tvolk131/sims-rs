use super::*;

#[test]
fn normal_frame_indicator_child_requires_exact_phase_tick_and_started_drain() {
    for phase in [
        FramePhase::Environment,
        FramePhase::Indicators,
        FramePhase::Classification,
    ] {
        for tick in [16, 17, 18] {
            for started in [false, true] {
                let frame = NormalFrame { tick: 17, phase };
                assert_eq!(
                    frame.require_indicator_child(tick, started).is_ok(),
                    phase == FramePhase::Indicators && tick == 17 && started,
                    "phase={phase:?}, tick={tick}, started={started}"
                );
            }
        }
    }
}

#[test]
fn normal_frame_indicator_scope_is_retained_without_repair_or_mutation() -> Result<()> {
    let frame = NormalFrame {
        tick: u64::MAX,
        phase: FramePhase::Indicators,
    };
    let bytes = serde_json::to_vec(&frame)?;
    let restored: NormalFrame = serde_json::from_slice(&bytes)?;
    restored.require_indicator_child(u64::MAX, true)?;
    ensure!(restored.require_indicator_child(0, true).is_err());
    ensure!(restored.require_indicator_child(u64::MAX, false).is_err());
    ensure!(serde_json::to_vec(&restored)? == bytes);
    Ok(())
}
