//! cTSWinMgr5daf06/5daf6c registration and removal. The clock sample comes
//! from5daf06's imported timeGetTime, not the simulation tick or CPState timer.
//! Dispatch of due entries is the ordered5da820 child in dispatch.
use serde::{Deserialize, Serialize};
pub mod dispatch;

/// 5db91c and its16-byte-record sort helpers use the same partition, insertion
/// and heap choices as the nearby-lot/presenter compiler sort. Compare signed
/// deadlines; equal deadlines retain that algorithm's order, not Rust sort's.
pub fn sort_entries(entries: &mut [Entry]) {
    crate::nearby_houses::sort_by(entries, &|a, b| (a.deadline as i32) < (b.deadline as i32));
}

/// 5da49f with a borrowed registry held stable by the UI owner. The original
/// locks its vector while searching, after the null/special-owner fast paths.
/// The host must provide synchronized access; this does not emulate OS locks.
pub fn registered(owner: u32, special: u32, owners: &[u32]) -> bool {
    owner != 0 && (owner == special || owners.contains(&owner))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub owner: u32,
    pub interval: u32,
    pub deadline: u32,
    pub argument: u32,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Registry {
    /// Manager+1cc..1d0, insertion order, including any historical duplicates.
    pub entries: Vec<Entry>,
    /// The two signed traversal indices+1d8/+1dc. Constructors/dispatch must
    /// supply these; registration must not reset or infer them.
    pub first: i32,
    pub second: i32,
}
impl Registry {
    pub fn register(&mut self, entry: Entry) -> Result<(), String> {
        if let Some(old) = self.entries.iter_mut().find(|e| e.owner == entry.owner) {
            *old = entry;
            return Ok(());
        }
        if self.entries.len() >= i32::MAX as usize {
            return Err("UI timer vector exceeds native signed index range".into());
        }
        self.entries
            .try_reserve(1)
            .map_err(|_| "UI timer allocation failed")?;
        self.entries.push(entry);
        Ok(())
    }
    /// Only the first match is removed. Indices equal to the removed position
    /// stay put; indices strictly after it decrement, using signed comparisons.
    pub fn remove(&mut self, owner: u32) -> Result<(), String> {
        if self.entries.len() > i32::MAX as usize {
            return Err("UI timer vector exceeds native signed index range".into());
        }
        if let Some(n) = self.entries.iter().position(|e| e.owner == owner) {
            let n = n as i32;
            if n < self.first {
                self.first = self.first.wrapping_sub(1);
            }
            if n < self.second {
                self.second = self.second.wrapping_sub(1);
            }
            self.entries.remove(n as usize);
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Stage {
    Clock,
    Waiting,
    Write(u32),
    Complete,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Registration {
    owner: u32,
    interval: u32,
    argument: u32,
    stage: Stage,
}
impl Registration {
    pub fn new(owner: u32, interval: u32, argument: u32) -> Self {
        Self {
            owner,
            interval,
            argument,
            stage: Stage::Clock,
        }
    }
    pub fn complete(&self) -> bool {
        self.stage == Stage::Complete
    }
    /// true requests one actual system-clock sample, including on replacement
    /// of an existing entry. The request remains pending until clock() returns.
    pub fn advance(&mut self, registry: &mut Registry) -> Result<bool, String> {
        match self.stage {
            Stage::Clock | Stage::Waiting => {
                self.stage = Stage::Waiting;
                Ok(true)
            }
            Stage::Write(now) => {
                registry.register(Entry {
                    owner: self.owner,
                    interval: self.interval,
                    deadline: now.wrapping_add(self.interval),
                    argument: self.argument,
                })?;
                self.stage = Stage::Complete;
                Ok(false)
            }
            Stage::Complete => Ok(false),
        }
    }
    pub fn clock(&mut self, now: u32) -> Result<(), String> {
        if self.stage != Stage::Waiting {
            return Err("UI timer clock reply has no caller".into());
        }
        self.stage = Stage::Write(now);
        Ok(())
    }
}
