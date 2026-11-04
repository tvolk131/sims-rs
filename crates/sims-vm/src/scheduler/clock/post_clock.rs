//! Application4d1ab8..4d1b23, after its elapsed child returns. Virtual, FPS
//! and profiling children remain explicit saved calls, never implicit replies.
use super::Timer;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fields {
    pub in_update: u8,
    pub post_clock_flag: u8,
    pub timer: Timer,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Request {
    PostClockCallback,
    ///473088, formatting and4d36de must finish before the profiler child.
    Fps,
    ///5d532f followed by4b3e5b; retains its external RDTSC/allocation owners.
    Profiler,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Stage {
    ClearFlag,
    AfterCallback,
    AfterFps,
    AfterProfiler,
    Complete,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Frame {
    stage: Stage,
    pending: Option<Request>,
}
impl Default for Frame {
    fn default() -> Self {
        Self {
            stage: Stage::ClearFlag,
            pending: None,
        }
    }
}
impl Frame {
    pub fn at_entry(&self) -> bool {
        self.stage == Stage::ClearFlag && self.pending.is_none()
    }
    pub fn complete(&self) -> bool {
        self.stage == Stage::Complete && self.pending.is_none()
    }
    pub fn request(&self) -> Option<Request> {
        self.pending
    }
    pub fn validate(&self) -> Result<(), String> {
        if let Some(request) = self.pending {
            let expected = match request {
                Request::PostClockCallback => Stage::AfterCallback,
                Request::Fps => Stage::AfterFps,
                Request::Profiler => Stage::AfterProfiler,
            };
            if self.stage != expected {
                return Err("post-clock child lost its saved return".into());
            }
        }
        Ok(())
    }
    pub fn acknowledge(&mut self, request: Request) -> Result<(), String> {
        self.validate()?;
        if self.pending != Some(request) {
            return Err("post-clock reply differs".into());
        }
        self.pending = None;
        Ok(())
    }
    pub fn advance(&mut self, fields: &mut Fields, now: u32) -> Result<Option<Request>, String> {
        self.validate()?;
        if self.pending.is_some() {
            return Ok(self.pending);
        }
        match self.stage {
            Stage::ClearFlag => {
                fields.in_update = 0;
                self.stage = Stage::AfterCallback;
                self.pending = Some(Request::PostClockCallback);
            }
            Stage::AfterCallback => {
                fields.timer.update(now);
                if fields.timer.consume() {
                    self.stage = Stage::AfterFps;
                    self.pending = Some(Request::Fps);
                } else {
                    self.stage = Stage::AfterProfiler;
                    self.pending = Some(Request::Profiler);
                }
            }
            Stage::AfterFps => {
                self.stage = Stage::AfterProfiler;
                self.pending = Some(Request::Profiler);
            }
            Stage::AfterProfiler => self.stage = Stage::Complete,
            Stage::Complete => (),
        }
        Ok(self.pending)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    #[cfg(feature = "private-fixtures")]
    #[test]
    fn actual_original_return_timer_cases_and_suspended_codecs() {
        let data: Value = serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/native-application-return.json"
        )))
        .unwrap();
        assert_eq!(
            data["source_receipt_sha256"],
            "16f919f7cb163e123923ede13bc136fc21cc568cdc0a1bf4b01822f188e1f487"
        );
        let cases = data["cases"].as_array().unwrap();
        assert_eq!(cases.len(), 73);
        for row in cases {
            let before = &row["before"];
            let timer: Vec<u32> = serde_json::from_value(before["timer"].clone()).unwrap();
            let mut fields = Fields {
                in_update: before["in_update"].as_u64().unwrap() as u8,
                post_clock_flag: before["post_clock_flag"].as_u64().unwrap() as u8,
                timer: Timer {
                    elapsed: timer[0],
                    interval: timer[1],
                    last: timer[2],
                    limit: timer[3],
                },
            };
            let now = before["now"].as_u64().unwrap() as u32;
            let mut frame = Frame::default();
            assert_eq!(
                frame.advance(&mut fields, now).unwrap(),
                Some(Request::PostClockCallback)
            );
            assert_eq!(fields.in_update, 0);
            // The captured concrete virtual5de715 is exactly this byte store.
            fields.post_clock_flag = 1;
            frame = serde_json::from_slice(&serde_json::to_vec(&frame).unwrap()).unwrap();
            let frozen = fields.clone();
            assert_eq!(
                frame.advance(&mut fields, now).unwrap(),
                Some(Request::PostClockCallback)
            );
            assert_eq!(fields, frozen);
            frame.acknowledge(Request::PostClockCallback).unwrap();
            let request = frame.advance(&mut fields, now).unwrap().unwrap();
            assert_eq!(
                request,
                if row["next_service"] == "fps" {
                    Request::Fps
                } else {
                    Request::Profiler
                },
                "{}",
                row["label"]
            );
            let words = [
                fields.timer.elapsed,
                fields.timer.interval,
                fields.timer.last,
                fields.timer.limit,
            ];
            assert_eq!(
                serde_json::to_value(words).unwrap(),
                row["after"]["timer"],
                "{}",
                row["label"]
            );
            assert_eq!(
                fields.in_update as u64,
                row["after"]["in_update"].as_u64().unwrap()
            );
            assert_eq!(
                fields.post_clock_flag as u64,
                row["after"]["post_clock_flag"].as_u64().unwrap()
            );
            frame = serde_json::from_slice(&serde_json::to_vec(&frame).unwrap()).unwrap();
            frame.acknowledge(request).unwrap();
            if request == Request::Fps {
                assert_eq!(
                    frame.advance(&mut fields, now).unwrap(),
                    Some(Request::Profiler)
                );
                frame.acknowledge(Request::Profiler).unwrap();
            }
            assert_eq!(frame.advance(&mut fields, now).unwrap(), None);
            assert!(frame.complete());
        }
    }

    #[test]
    fn wrong_reply_or_corrupt_stage_preserves_fields_and_pending_owner() {
        let mut frame = Frame::default();
        let mut fields = Fields {
            in_update: 1,
            post_clock_flag: 1,
            timer: Timer::default(),
        };
        frame.advance(&mut fields, 1).unwrap();
        let before = frame.clone();
        assert!(frame.acknowledge(Request::Profiler).is_err());
        assert_eq!(frame, before);
        frame.stage = Stage::AfterFps;
        let before = fields.clone();
        assert!(frame.advance(&mut fields, 2).is_err());
        assert_eq!(fields, before);
    }
}
