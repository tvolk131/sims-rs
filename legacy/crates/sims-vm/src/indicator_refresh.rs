//! 4448a6/444962 notification queues and 4464db ordered refresh drain.
//!
//! A drain belongs at the native presenter boundary, not at every VM slice.
//! Requests are synchronous services; suspending one must retain this cursor
//! and the queues. Each phase captures its end once but reads entries live.
//! Reentrant erasure/reallocation invalidating native iterators is unsupported.
use crate::{ObjectId, indicator_unregister::Queues};
use serde::{Deserialize, Serialize};

struct People<'a>(&'a std::collections::BTreeMap<ObjectId, crate::Object>);
impl View for People<'_> {
    fn date_target(&self, person: ObjectId) -> Result<i16, String> {
        self.0
            .get(&person)
            .and_then(|o| o.person.as_ref())
            .and_then(|p| p.get(&77))
            .copied()
            .ok_or_else(|| "native date notification person word absent".into())
    }
    fn script_id(&self, person: ObjectId) -> Result<i16, String> {
        self.0
            .get(&person)
            .and_then(|o| o.data.get(&11))
            .copied()
            .ok_or_else(|| "native notification script ID absent".into())
    }
}
impl crate::World {
    pub(crate) fn notify_indicator_word(
        &mut self,
        person: ObjectId,
        field: i16,
    ) -> Result<(), String> {
        match field {
            77 => date(
                &mut self.indicator_queues,
                Some(person),
                &People(&self.objects),
            ),
            81 => star(&mut self.indicator_queues, Some(person)),
            _ => Err("unexpected native indicator word notification".into()),
        }
    }
    pub fn validate_indicator_queues(&self) -> Result<(), String> {
        if self
            .indicator_queues
            .dates
            .iter()
            .chain(&self.indicator_queues.stars)
            .flatten()
            .any(|id| *id == 0 || !self.objects.contains_key(id))
        {
            return Err("native indicator refresh queue references an absent object".into());
        }
        Ok(())
    }
}

pub trait View {
    fn date_target(&self, person: ObjectId) -> Result<i16, String>;
    fn script_id(&self, person: ObjectId) -> Result<i16, String>;
}

/// Called at memory-resolution write intent, before the person word is written.
pub fn date(queues: &mut Queues, person: Option<ObjectId>, view: &impl View) -> Result<(), String> {
    let Some(person) = person else { return Ok(()) };
    if person == 0 {
        return Err("null native date notification identity".into());
    }
    if view.date_target(person)? == 0 {
        return Ok(());
    }
    for queued in queues.dates.iter().flatten() {
        if view.script_id(*queued)? == view.script_id(person)? {
            return Ok(());
        }
    }
    queues.dates.push(Some(person));
    queues.dirty = true;
    Ok(())
}

/// Unlike date notifications, stars preserve duplicates and dirty even on null.
pub fn star(queues: &mut Queues, person: Option<ObjectId>) -> Result<(), String> {
    if person == Some(0) {
        return Err("null native star notification identity".into());
    }
    if person.is_some() {
        queues.stars.push(person);
    }
    queues.dirty = true;
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Request {
    Date(Option<ObjectId>),
    Star(Option<ObjectId>),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Stage {
    Start,
    Dates,
    BeginStars,
    Stars,
    Done,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Drain {
    stage: Stage,
    index: usize,
    end: usize,
    pending: Option<Request>,
}
impl Default for Drain {
    fn default() -> Self {
        Self {
            stage: Stage::Start,
            index: 0,
            end: 0,
            pending: None,
        }
    }
}
impl Drain {
    pub fn pending(&self) -> Option<Request> {
        self.pending
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.index > self.end
            || (!matches!(self.stage, Stage::Dates | Stage::Stars)
                && (self.index != 0 || self.end != 0))
            || self.pending.is_some_and(|r| {
                self.index == self.end
                    || matches!(r, Request::Date(Some(0)) | Request::Star(Some(0)))
                    || !matches!(
                        (self.stage, r),
                        (Stage::Dates, Request::Date(_)) | (Stage::Stars, Request::Star(_))
                    )
            })
        {
            return Err("invalid native indicator drain continuation".into());
        }
        Ok(())
    }
    pub fn advance(&mut self, queues: &mut Queues) -> Result<Option<Request>, String> {
        self.validate()?;
        if self.pending.is_some() {
            return Ok(self.pending);
        }
        loop {
            match self.stage {
                Stage::Start => {
                    self.stage = if queues.dirty {
                        Stage::Dates
                    } else {
                        Stage::Done
                    };
                    if queues.dirty {
                        self.end = queues.dates.len();
                    }
                }
                Stage::Dates | Stage::Stars => {
                    let dates = self.stage == Stage::Dates;
                    let items = if dates {
                        &mut queues.dates
                    } else {
                        &mut queues.stars
                    };
                    if items.len() < self.end {
                        return Err("native refresh callback invalidated queue iterators".into());
                    }
                    if self.index < self.end {
                        let person = items[self.index];
                        if person == Some(0) {
                            return Err("null native refresh queue identity".into());
                        }
                        self.pending = Some(if dates {
                            Request::Date(person)
                        } else {
                            Request::Star(person)
                        });
                        return Ok(self.pending);
                    }
                    // 448847 clears the live range, including additions after
                    // the captured end. It is not a pop-front work queue.
                    items.clear();
                    self.index = 0;
                    self.end = 0;
                    self.stage = if dates {
                        Stage::BeginStars
                    } else {
                        Stage::Done
                    };
                    if !dates {
                        queues.dirty = false;
                    }
                }
                Stage::BeginStars => {
                    self.end = queues.stars.len();
                    self.stage = Stage::Stars;
                }
                Stage::Done => return Ok(None),
            }
        }
    }
    pub fn acknowledge(&mut self, request: Request) -> Result<(), String> {
        self.validate()?;
        if self.pending != Some(request) {
            return Err("stale native indicator drain acknowledgment".into());
        }
        self.index += 1;
        self.pending = None;
        Ok(())
    }
}
