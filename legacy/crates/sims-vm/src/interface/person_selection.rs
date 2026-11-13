//!4e989b: portrait index -> resource instance -> lot restrictions ->445449.
//!Returning without selection differs from calling445449 with a null target.
use crate::{ObjectId, image_cache};
use serde::{Deserialize, Serialize};

pub trait View {
    fn cache_guid(&self, index: u32) -> Result<u32, String>;
    fn registry_present(&self) -> bool;
    ///4408d4/445857 can lazily populate the resource's ordered instance vector.
    fn first_instance(&mut self, guid: u32) -> Result<Option<ObjectId>, String>;
    fn zoning(&self) -> Result<i32, String>;
    fn lot_number(&self) -> Result<i16, String>;
    fn selection_override(&self) -> Result<u32, String>;
    fn person_data(&self, object: ObjectId, index: i16) -> Result<i16, String>;
    fn controlled_family(&self) -> Result<i32, String>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Decision {
    Return,
    Select(Option<ObjectId>),
}

///4e8c35 uses an unsigned index; -1 and other out-of-range values return zero.
pub fn cache_guid(cache: &image_cache::State, index: u32) -> u32 {
    cache.entries.get(index as usize).map_or(0, |e| e.guid)
}
///4e9753 returns the first matching entry, or -1. No cache is reconstructed.
pub fn cache_index(cache: &image_cache::State, guid: u32) -> i32 {
    cache
        .entries
        .iter()
        .position(|e| e.guid == guid)
        .map_or(-1, |i| i as i32)
}

pub fn select(view: &mut impl View, index: u32) -> Result<Decision, String> {
    let guid = view.cache_guid(index)?;
    if !view.registry_present() {
        return Ok(Decision::Return);
    }
    let target = if guid == 0 {
        None
    } else {
        view.first_instance(guid)?
    };
    let Some(person) = target else {
        return Ok(Decision::Select(None));
    };
    //40745f tests neighborhood zoning, not Lot+50 or a guessed home range.
    let allowed = if view.zoning()? == 1 {
        view.selection_override()? != 0 || view.person_data(person, 32)? == 0
    } else if (21..=30).contains(&view.lot_number()?) {
        view.selection_override()? != 0 || view.person_data(person, 32)? != 2
    } else {
        //45e2db:40747c, then4074c8. A false result returns without445449.
        if view.zoning()? != 2 && !(40..=49).contains(&view.lot_number()?) {
            return Ok(Decision::Return);
        }
        view.selection_override()? != 0
            || i32::from(view.person_data(person, 61)?) == view.controlled_family()?
            || view.person_data(person, 32)? != 2
    };
    Ok(Decision::Select(allowed.then_some(person)))
}
