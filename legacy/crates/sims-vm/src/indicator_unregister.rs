//! 447d8b synchronous-removal prelude and 448097 indicator unregistration.
//! This is not 4481eb (release to a pool), nor the no-op 447d80 unlink hook.
//! UI callbacks complete synchronously; all later inputs are read again.
use crate::{ObjectId, indicator_pools::Pools, load_indicators::Indicators};
use serde::{Deserialize, Serialize};

/// Registry +b0 and +bc. These are pending date/star refreshes, not person order.
/// Erase preserves order, nulls and duplicate entries after the first match.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Queues {
    pub dates: Vec<Option<ObjectId>>,
    pub stars: Vec<Option<ObjectId>>,
    /// Registry +ac. Null star notifications set this even with empty vectors.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub dirty: bool,
}
impl Queues {
    pub fn is_empty(&self) -> bool {
        !self.dirty && self.dates.is_empty() && self.stars.is_empty()
    }
}
pub trait Host {
    fn enabled(&self) -> bool;
    fn kind(&self, object: ObjectId) -> Result<i16, String>;
    fn guid(&self, object: ObjectId) -> Result<u32, String>;
    fn cast_person(&mut self, object: ObjectId) -> Result<Option<ObjectId>, String>;
    fn preview_person(&mut self) -> Result<Option<ObjectId>, String>;
    fn held_object(&mut self) -> Result<Option<ObjectId>, String>;
    /// 4d3f17: dispatch -1 to the two extant UI owners. Does not itself assign
    /// the preview/held getters, so callers must not clear those preemptively.
    fn clear_ui(&mut self) -> Result<(), String>;
    fn indicators(&mut self) -> (&mut Indicators, &mut Pools);
    fn queues(&mut self) -> &mut Queues;
}
fn erase_first(items: &mut Vec<Option<ObjectId>>, object: Option<ObjectId>) {
    if let Some(index) = items.iter().position(|&id| id == object) {
        items.remove(index);
    }
}

pub fn indicator(host: &mut impl Host, target: Option<ObjectId>) {
    let Some(target) = target.filter(|_| host.enabled()) else {
        return;
    };
    let (indicators, pools) = host.indicators();
    if indicators.primary == Some(target) {
        indicators.primary = None;
    }
    if indicators.secondary == Some(target) {
        indicators.secondary = None;
    }
    erase_first(&mut pools.active, Some(target));
    erase_first(&mut pools.spare, Some(target));
}

pub fn unregister(host: &mut impl Host, target: Option<ObjectId>) -> Result<(), String> {
    let Some(target) = target.filter(|_| host.enabled()) else {
        return Ok(());
    };
    if host.kind(target)? == 2 {
        let preview = host.preview_person()?;
        // Native RTTI runs even when preview is null.
        let person = host.cast_person(target)?;
        if preview.is_some() && person == preview {
            host.clear_ui()?;
        }
    }
    if host.held_object()? == Some(target) {
        host.clear_ui()?;
    }
    if matches!(host.guid(target)?, 0x2f726fff | 0x0baa71c5 | 0xe9337020) {
        // This helper checks live enabled again after UI callbacks.
        indicator(host, Some(target));
    }
    if !host.queues().dates.is_empty() && host.kind(target)? == 2 {
        let person = host.cast_person(target)?;
        erase_first(&mut host.queues().dates, person);
    }
    if !host.queues().stars.is_empty() && host.kind(target)? == 2 {
        let person = host.cast_person(target)?;
        erase_first(&mut host.queues().stars, person);
    }
    Ok(())
}
