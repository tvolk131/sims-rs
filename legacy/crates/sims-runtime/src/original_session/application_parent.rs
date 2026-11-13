//! Qualified4d1901 caller suspended in46d9d6. No frame entry is replayed.
//! Source-bound post-clock timer/callback work can resume. Profiling borrows
//! its shared process owner and explicit timestamp; FPS/outer calls stay separate.
use super::{clock, completed_application::Fields};
use anyhow::{Context, Result, ensure};
pub use post_clock::Request;
use serde::{Deserialize, Serialize};
use sims_vm::scheduler::clock::{Timer, post_clock, profiler};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Profile {
    GothCompleted88448Application20261029,
    GothCompleted89073Application20261031,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Phase {
    /// Saved native return4d1ab8 has not been reached:46d9d6 still owns control.
    Elapsed,
    /// At4d1ab8 before clearing receiver+118. No later callback was dispatched.
    AfterElapsed,
    Returning,
    AfterApplication,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct ReturnOwner {
    timer: Timer,
    frame: post_clock::Frame,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    profile: Profile,
    fields: Fields,
    phase: Phase,
    /// Older partial captures did not retain this timer. Absence never grants
    /// permission to synthesize one when the elapsed caller returns.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    return_owner: Option<ReturnOwner>,
}
impl State {
    pub(super) fn from_completed_inputs(
        fields: Fields,
        clock: &clock::State,
        tick: u64,
    ) -> Result<Self> {
        Self::from_profile(
            fields,
            clock,
            tick,
            Profile::GothCompleted88448Application20261029,
        )
    }
    pub(super) fn from_fresh_completed_inputs(
        fields: Fields,
        clock: &clock::State,
        tick: u64,
    ) -> Result<Self> {
        ensure!(
            tick == 89073,
            "fresh application initial caller cut differs"
        );
        Self::from_profile(
            fields,
            clock,
            tick,
            Profile::GothCompleted89073Application20261031,
        )
    }
    fn from_profile(
        fields: Fields,
        clock: &clock::State,
        tick: u64,
        profile: Profile,
    ) -> Result<Self> {
        ensure!(
            clock.at_completed_update(),
            "application import requires the actual completed clock child"
        );
        let state = Self {
            profile,
            fields,
            phase: Phase::Elapsed,
            return_owner: None,
        };
        state.validate(Some(clock), tick)?;
        Ok(state)
    }
    pub fn fields(&self) -> &Fields {
        &self.fields
    }
    pub fn awaiting_elapsed(&self) -> bool {
        self.phase == Phase::Elapsed
    }
    pub fn at_elapsed_return(&self) -> bool {
        self.phase == Phase::AfterElapsed
    }
    pub(super) fn ready_for_enclosing_return(&self) -> bool {
        self.profile == Profile::GothCompleted89073Application20261031
            && self.phase == Phase::AfterApplication
    }
    pub fn post_clock_timer(&self) -> Option<Timer> {
        self.return_owner.as_ref().map(|r| r.timer)
    }
    pub fn return_request(&self) -> Option<Request> {
        self.return_owner.as_ref().and_then(|r| r.frame.request())
    }
    fn complete_profiler(&mut self, buffer: &mut profiler::Buffer, timestamp: u64) -> Result<()> {
        ensure!(
            self.profile == Profile::GothCompleted89073Application20261031
                && self.phase == Phase::Returning,
            "profiler reply lost its application caller"
        );
        let owner = self
            .return_owner
            .as_mut()
            .context("profiler reply lost its saved return")?;
        owner.frame.validate().map_err(anyhow::Error::msg)?;
        ensure!(
            owner.frame.request() == Some(Request::Profiler),
            "profiler is not the pending application child"
        );
        // Validate the saved return before changing the separate shared owner.
        // A full buffer stops here; it never acknowledges an unexecuted allocator.
        buffer
            .append(profiler::Record {
                timestamp,
                name: 0x6c7c3c,
                argument: 0,
            })
            .map_err(anyhow::Error::msg)?;
        owner
            .frame
            .acknowledge(Request::Profiler)
            .map_err(anyhow::Error::msg)?;
        Ok(())
    }
    pub(super) fn bind_return_timer(&mut self, timer: Timer) -> Result<()> {
        ensure!(
            self.profile == Profile::GothCompleted89073Application20261031
                && self.phase == Phase::Elapsed
                && self.return_owner.is_none(),
            "post-clock timer lost its fresh source binding"
        );
        self.return_owner = Some(ReturnOwner {
            timer,
            frame: post_clock::Frame::default(),
        });
        Ok(())
    }
    fn advance_return(&mut self, now: u32) -> Result<Option<Request>> {
        ensure!(
            matches!(
                self.phase,
                Phase::AfterElapsed | Phase::Returning | Phase::AfterApplication
            ),
            "elapsed child still owns application return"
        );
        let owner = self
            .return_owner
            .as_mut()
            .context("application return timer/source owner absent")?;
        ensure!(
            self.profile == Profile::GothCompleted89073Application20261031,
            "application return virtual profile is not qualified"
        );
        let mut fields = post_clock::Fields {
            in_update: self.fields.in_update,
            post_clock_flag: self.fields.post_clock_flag,
            timer: owner.timer,
        };
        for _ in 0..3 {
            let request = owner
                .frame
                .advance(&mut fields, now)
                .map_err(anyhow::Error::msg)?;
            self.fields.in_update = fields.in_update;
            self.fields.post_clock_flag = fields.post_clock_flag;
            owner.timer = fields.timer;
            self.phase = if owner.frame.complete() {
                Phase::AfterApplication
            } else {
                Phase::Returning
            };
            match request {
                Some(Request::PostClockCallback) => {
                    // The source-bound concrete virtual5de715 is exactly
                    // MOV BYTE PTR [ECX+60],1; RET, with no nested service.
                    fields.post_clock_flag = 1;
                    self.fields.post_clock_flag = 1;
                    owner
                        .frame
                        .acknowledge(Request::PostClockCallback)
                        .map_err(anyhow::Error::msg)?;
                }
                other => return Ok(other),
            }
        }
        anyhow::bail!("application return work bound exceeded")
    }
    pub(super) fn clock_returned(&mut self) -> Result<()> {
        ensure!(
            self.awaiting_elapsed(),
            "application clock return has no retained caller"
        );
        self.phase = Phase::AfterElapsed;
        Ok(())
    }
    pub(super) fn validate(&self, child: Option<&clock::State>, tick: u64) -> Result<()> {
        let child = child.context("retained application parent lost its elapsed clock")?;
        let (first_tick, source) = match self.profile {
            Profile::GothCompleted88448Application20261029 => (
                88448,
                clock::CapturedCode::Goth88448DormantProtectedPoll20261029,
            ),
            Profile::GothCompleted89073Application20261031 => (
                89073,
                clock::CapturedCode::Goth89073DormantProtectedPoll20261031,
            ),
        };
        ensure!(
            tick >= first_tick && child.captured_code == Some(source),
            "retained application and clock source profiles differ"
        );
        let expected_update = if matches!(self.phase, Phase::Returning | Phase::AfterApplication) {
            0
        } else {
            1
        };
        ensure!(
            self.fields
                == (Fields {
                    in_update: expected_update,
                    suppressed: 0,
                    post_clock_flag: 1
                }),
            "retained application flags changed before their native return instructions"
        );
        ensure!(
            self.awaiting_elapsed() == child.pending(),
            "retained application/elapsed continuation order differs"
        );
        if let Some(owner) = &self.return_owner {
            ensure!(
                self.profile == Profile::GothCompleted89073Application20261031,
                "post-clock source profile differs"
            );
            owner.frame.validate().map_err(anyhow::Error::msg)?;
            ensure!(
                match self.phase {
                    Phase::Elapsed | Phase::AfterElapsed => owner.frame.at_entry(),
                    Phase::Returning => !owner.frame.at_entry() && !owner.frame.complete(),
                    Phase::AfterApplication => owner.frame.complete(),
                },
                "post-clock cursor differs from its application caller"
            );
        } else {
            ensure!(
                matches!(self.phase, Phase::Elapsed | Phase::AfterElapsed),
                "application return advanced without its retained timer"
            );
        }
        Ok(())
    }
}

impl super::Session {
    /// Complete the original5d532f/4b3e5b child with explicit RDTSC input.
    /// `buffer` is a shared process owner, saved alongside this session, not
    /// recreated per application frame. No host clock or RNG is read here.
    pub fn complete_application_profiler(
        &mut self,
        buffer: &mut profiler::Buffer,
        timestamp: u64,
    ) -> Result<()> {
        let clock = self
            .host
            .presentation
            .clock
            .as_ref()
            .context("application clock absent")?;
        clock.validate(self)?;
        let parent = self
            .host
            .presentation
            .application_parent
            .as_ref()
            .context("application parent absent")?;
        parent.validate(Some(clock), self.simulation.vm.tick)?;
        ensure!(!clock.pending(), "elapsed child still owns control");
        self.host
            .presentation
            .application_parent
            .as_mut()
            .unwrap()
            .complete_profiler(buffer, timestamp)
    }
    /// Advance the source-bound return from46d9d6. FPS/window/profiler children
    /// remain pending services; this cannot authorize a new application frame.
    pub fn advance_application_return(&mut self) -> Result<Option<Request>> {
        let clock = self
            .host
            .presentation
            .clock
            .as_ref()
            .context("application clock absent")?;
        clock.validate(self)?;
        let parent = self
            .host
            .presentation
            .application_parent
            .as_ref()
            .context("application parent absent")?;
        parent.validate(Some(clock), self.simulation.vm.tick)?;
        ensure!(!clock.pending(), "elapsed child still owns control");
        let now = clock.now;
        let result = self
            .host
            .presentation
            .application_parent
            .as_mut()
            .unwrap()
            .advance_return(now)?;
        self.host
            .presentation
            .application_parent
            .as_ref()
            .unwrap()
            .validate(
                self.host.presentation.clock.as_ref(),
                self.simulation.vm.tick,
            )?;
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::original_session::{Saved, completed_application};
    use serde_json::{Value, json};
    use sims_fixtures::Store;
    use std::{fs, path::Path};
    #[cfg(feature = "private-fixtures")]
    #[test]
    fn post_clock_timer_and_profile_service_survive_resume_without_repeating_work() -> Result<()> {
        let mut parent = State {
            profile: Profile::GothCompleted89073Application20261031,
            fields: Fields {
                in_update: 1,
                suppressed: 0,
                post_clock_flag: 1,
            },
            phase: Phase::AfterElapsed,
            return_owner: Some(ReturnOwner {
                timer: Timer {
                    elapsed: 607,
                    interval: 1000,
                    last: 584114,
                    limit: 0,
                },
                frame: post_clock::Frame::default(),
            }),
        };
        assert_eq!(parent.advance_return(584466)?, Some(Request::Profiler));
        assert_eq!(
            parent.fields,
            Fields {
                in_update: 0,
                suppressed: 0,
                post_clock_flag: 1
            }
        );
        assert_eq!(
            parent.post_clock_timer(),
            Some(Timer {
                elapsed: 959,
                interval: 1000,
                last: 584466,
                limit: 0
            })
        );
        let frozen = parent.clone();
        let mut cold: State = serde_json::from_slice(&serde_json::to_vec(&parent)?)?;
        assert_eq!(cold.advance_return(586466)?, Some(Request::Profiler));
        assert_eq!(
            cold, frozen,
            "pending profiler must retain the earlier timer sample"
        );
        let native: Value = serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../sims-vm/tests/fixtures/native-profiler-append.json"
        )))?;
        let records: Vec<[u32; 4]> = serde_json::from_value(native["before"].clone())?;
        let buffer = profiler::Buffer::from_captured(
            records
                .into_iter()
                .map(profiler::Record::from_words)
                .collect(),
            32,
        )
        .map_err(anyhow::Error::msg)?;
        let serialized = postcard::to_allocvec(&(cold, buffer))?;
        let (mut resumed, mut buffer): (State, profiler::Buffer) =
            postcard::from_bytes(&serialized)?;
        let mut full = profiler::Buffer::from_captured(buffer.records().to_vec(), 3)
            .map_err(anyhow::Error::msg)?;
        let unchanged = serde_json::to_vec(&(&resumed, &full))?;
        assert!(resumed.complete_profiler(&mut full, 0).is_err());
        assert_eq!(serde_json::to_vec(&(&resumed, &full))?, unchanged);
        let sampled: [u32; 2] = serde_json::from_value(native["sampled_clock_words"].clone())?;
        resumed.complete_profiler(
            &mut buffer,
            u64::from(sampled[0]) | (u64::from(sampled[1]) << 32),
        )?;
        let expected: Vec<[u32; 4]> = serde_json::from_value(native["after"].clone())?;
        assert_eq!(
            buffer
                .records()
                .iter()
                .copied()
                .map(profiler::Record::words)
                .collect::<Vec<_>>(),
            expected
        );
        let unchanged = serde_json::to_vec(&(&resumed, &buffer))?;
        assert!(resumed.complete_profiler(&mut buffer, 0).is_err());
        assert_eq!(serde_json::to_vec(&(&resumed, &buffer))?, unchanged);
        let (mut resumed, buffer): (State, profiler::Buffer) = serde_json::from_slice(&unchanged)?;
        assert_eq!(resumed.advance_return(586466)?, None);
        assert_eq!(resumed.phase, Phase::AfterApplication);
        assert_eq!(resumed.post_clock_timer(), parent.post_clock_timer());
        assert_eq!(buffer.records().len(), 4);
        let mut absent = parent.clone();
        absent.return_owner = None;
        let frozen = absent.clone();
        assert!(absent.advance_return(586466).is_err());
        assert_eq!(absent, frozen);
        let mut early = parent;
        early.phase = Phase::Elapsed;
        let frozen = early.clone();
        assert!(early.advance_return(586466).is_err());
        assert_eq!(early, frozen);
        Ok(())
    }
    #[test]
    #[ignore = "private original application parent and retained managed checkpoint"]
    fn retained_parent_codec_child_order_and_old_checkpoint_absence() -> Result<()> {
        std::thread::Builder::new()
            .stack_size(16 << 20)
            .spawn(actual)?
            .join()
            .map_err(|_| anyhow::anyhow!("application parent test worker panicked"))?
    }
    fn actual() -> Result<()> {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let capsule = root.join("fixtures/private/goth-supplemental-owners-20261029-first");
        let refs: Value =
            serde_json::from_slice(&fs::read(capsule.join("session-tools-first/refs.json"))?)?;
        let mut saved: Saved = Store::open(capsule.join("session-tools-first/store"))?
            .get_value(&serde_json::from_value(refs["session"].clone())?)?;
        ensure!(
            saved.view.application_parent.is_none(),
            "historical checkpoint invented application ownership"
        );
        let vm = postcard::to_allocvec(&saved.simulation.state.vm)?;
        let source: Value = serde_json::from_slice(&fs::read(
            capsule.join("application-parent-fourth/before.json"),
        )?)?;
        let clock = saved.view.clock.as_ref().context("retained child absent")?;
        let parent = completed_application::import(&source)?.runtime_parent(clock, 88448)?;
        parent.validate(Some(clock), 88448)?;
        ensure!(
            parent.validate(None, 88448).is_err(),
            "missing child admitted"
        );
        let mut false_flags = parent.clone();
        false_flags.fields.in_update = 0;
        ensure!(
            false_flags.validate(Some(clock), 88448).is_err(),
            "early native return write admitted"
        );
        let mut wrong_profile = clock.clone();
        wrong_profile.captured_code = None;
        ensure!(
            parent.validate(Some(&wrong_profile), 88448).is_err(),
            "clock source mismatch admitted"
        );
        let mut returned = parent.clone();
        returned.clock_returned()?;
        ensure!(
            returned.validate(Some(clock), 88448).is_err(),
            "returned caller with live child admitted"
        );
        let mut finished = serde_json::to_value(clock)?;
        finished["call"] = json!(null);
        let finished: clock::State = serde_json::from_value(finished)?;
        returned.validate(Some(&finished), 88448)?;
        ensure!(
            returned.clock_returned().is_err(),
            "duplicate child return admitted"
        );
        ensure!(
            completed_application::import(&source)?
                .runtime_parent(&finished, 88448)
                .is_err(),
            "idle child reconstructed as original active call"
        );
        saved.view.application_parent = Some(parent.clone());
        let temporary = std::env::temp_dir().join(format!(
            "sims-application-parent-codec-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_nanos()
        ));
        let codec = Store::create(&temporary)?;
        let reference = codec.put_value(&saved)?;
        let restored: Saved = codec.get_value(&reference)?;
        ensure!(
            restored.view.application_parent == Some(parent),
            "retained caller cold codec changed fields or phase"
        );
        ensure!(
            postcard::to_allocvec(&restored.simulation.state.vm)? == vm,
            "parent import or codec changed VM/RNG"
        );
        fs::remove_dir_all(temporary)?;
        Ok(())
    }
}
