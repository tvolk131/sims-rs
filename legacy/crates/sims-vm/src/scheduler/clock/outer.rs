//! Saved5d5e7c..5d5ec2 after the application child returns. The periodic
//! registry, rendering and profiling calls remain ordered, explicit children.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Counters {
    pub frames: u32,
    pub periodic: u32,
}
impl Counters {
    pub fn from_words([frames, periodic]: [u32; 2]) -> Self {
        Self { frames, periodic }
    }
    pub fn words(self) -> [u32; 2] {
        [self.frames, self.periodic]
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Request {
    ///603021 receives the old periodic counter, after both words increment.
    Periodic {
        ordinal: u32,
    },
    Render,
    Profiler,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Stage {
    AfterApplication,
    AfterPeriodic,
    AfterRender,
    AfterProfiler,
    Complete,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Frame {
    stage: Stage,
    pending: Option<Request>,
}
impl Frame {
    /// At5d5e7c, not at outer-frame entry. Does not sample application time.
    pub fn at_application_return() -> Self {
        Self {
            stage: Stage::AfterApplication,
            pending: None,
        }
    }
    pub fn complete(&self) -> bool {
        self.stage == Stage::Complete && self.pending.is_none()
    }
    pub fn pending(&self) -> Option<Request> {
        self.pending
    }
    pub fn at_entry(&self) -> bool {
        self.stage == Stage::AfterApplication && self.pending.is_none()
    }
    pub fn validate(&self) -> Result<(), String> {
        if let Some(request) = self.pending {
            let stage = match request {
                Request::Periodic { .. } => Stage::AfterPeriodic,
                Request::Render => Stage::AfterRender,
                Request::Profiler => Stage::AfterProfiler,
            };
            if self.stage != stage {
                return Err("outer child lost its saved return".into());
            }
        }
        Ok(())
    }
    pub fn advance(&mut self, counters: &mut Counters) -> Result<Option<Request>, String> {
        self.validate()?;
        if self.pending.is_some() {
            return Ok(self.pending);
        }
        self.pending = match self.stage {
            Stage::AfterApplication => {
                let old = counters.frames;
                counters.frames = counters.frames.wrapping_add(1);
                if old & 3 == 0 {
                    let ordinal = counters.periodic;
                    counters.periodic = counters.periodic.wrapping_add(1);
                    self.stage = Stage::AfterPeriodic;
                    Some(Request::Periodic { ordinal })
                } else {
                    self.stage = Stage::AfterRender;
                    Some(Request::Render)
                }
            }
            Stage::AfterPeriodic => {
                self.stage = Stage::AfterRender;
                Some(Request::Render)
            }
            Stage::AfterRender => {
                self.stage = Stage::AfterProfiler;
                Some(Request::Profiler)
            }
            Stage::AfterProfiler | Stage::Complete => {
                self.stage = Stage::Complete;
                None
            }
        };
        Ok(self.pending)
    }
    pub fn acknowledge(&mut self, request: Request) -> Result<(), String> {
        self.validate()?;
        if self.pending != Some(request) {
            return Err("outer return differs from pending child".into());
        }
        self.pending = None;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[derive(Deserialize)]
    struct Case {
        before: [u32; 2],
        after: [u32; 2],
        next_child: String,
        ordinal: Option<u32>,
    }
    #[derive(Deserialize)]
    struct Oracle {
        cases: Vec<Case>,
    }
    #[cfg(feature = "private-fixtures")]
    #[test]
    fn native_prefix_and_suspended_child_order() -> Result<(), Box<dyn std::error::Error>> {
        let oracle: Oracle = serde_json::from_str(include_str!(
            "../../../tests/fixtures/native-outer-prefix.json"
        ))?;
        assert_eq!(oracle.cases.len(), 68);
        for (i, case) in oracle.cases.into_iter().enumerate() {
            let mut counts = Counters::from_words(case.before);
            let mut frame = Frame::at_application_return();
            let expected = if case.next_child == "periodic" {
                Request::Periodic {
                    ordinal: case.ordinal.unwrap(),
                }
            } else {
                Request::Render
            };
            assert_eq!(frame.advance(&mut counts)?, Some(expected), "case{i}");
            assert_eq!(counts.words(), case.after);
            let mut frozen = serde_json::to_vec(&(&frame, counts))?;
            let (mut frame, mut counts): (Frame, Counters) = serde_json::from_slice(&frozen)?;
            assert_eq!(frame.advance(&mut counts)?, Some(expected));
            assert_eq!(serde_json::to_vec(&(&frame, counts))?, frozen);
            assert!(frame.acknowledge(Request::Profiler).is_err());
            assert_eq!(serde_json::to_vec(&(&frame, counts))?, frozen);
            if let Request::Periodic { ordinal } = expected {
                assert!(
                    frame
                        .acknowledge(Request::Periodic {
                            ordinal: ordinal.wrapping_add(1)
                        })
                        .is_err()
                );
                frame.acknowledge(expected)?;
                assert_eq!(frame.advance(&mut counts)?, Some(Request::Render));
            }
            frame.acknowledge(Request::Render)?;
            assert_eq!(frame.advance(&mut counts)?, Some(Request::Profiler));
            frozen = serde_json::to_vec(&(&frame, counts))?;
            let (mut frame, mut counts): (Frame, Counters) = serde_json::from_slice(&frozen)?;
            frame.acknowledge(Request::Profiler)?;
            assert_eq!(frame.advance(&mut counts)?, None);
            assert!(frame.complete());
            assert_eq!(counts.words(), case.after);
            assert!(frame.acknowledge(Request::Profiler).is_err());
        }
        let mut invalid = serde_json::to_value(Frame::at_application_return())?;
        invalid["pending"] = serde_json::to_value(Request::Render)?;
        let mut invalid: Frame = serde_json::from_value(invalid)?;
        let mut counts = Counters::from_words([0, 0]);
        let before = serde_json::to_vec(&(&invalid, counts))?;
        assert!(invalid.advance(&mut counts).is_err());
        assert_eq!(serde_json::to_vec(&(&invalid, counts))?, before);
        Ok(())
    }
}
