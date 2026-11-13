//! Original headline state. Rendering and portrait composition are
//! separate services; the VM preserves priority, expiry, and animation state.
use crate::{Program, Thread, Vm};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Catalog {
    pub source: String,
    pub content_hash: String,
    pub frames: BTreeMap<u16, u32>,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct State {
    pub duration: i32,
    pub sprite: u16,
    /// 46dfe9 stores a resource pointer with sprite -1, not an owned bitmap.
    /// GUIDs preserve that identity independently of live-object lifetimes.
    /// None with sprite -1 is the native null catalog-resource result.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub portrait: Option<u32>,
    pub frames: i32,
    pub frame: i32,
    pub step: i32,
    pub ticks: i32,
    pub background: u16,
    pub overlay: u16,
    pub priority: i32,
    pub flag: bool,
    pub last_update: Option<u64>,
}
/// Original category-to-SPR# mapping (0x416a6f), including the old category-0 IDs.
pub fn sprite(kind: u8, index: i32) -> u16 {
    match kind {
        0 => match index {
            0..=13 => 200 + index as u16,
            14..=18 => 400 + (index - 14) as u16,
            _ => 0,
        },
        1..=6 | 8..=10 => {
            let base = [0, 100, 400, 200, 300, 500, 600, 0, 700, 800, 900][usize::from(kind)];
            (base + index) as u16
        }
        _ => 0,
    }
}
impl State {
    /// Priority and clear happen before any resource or local-variable lookup.
    fn begin(&mut self, operands: &[u8; 8]) -> bool {
        let priority = if operands[3] == 4 { 2 } else { 1 };
        if self.duration != 0 && priority < self.priority {
            return false;
        }
        self.priority = priority;
        if i16::from_le_bytes([operands[4], operands[5]]) == 0 {
            self.duration = 0;
            self.ticks = 0;
            return false;
        }
        true
    }
    pub fn apply(
        &mut self,
        operands: &[u8; 8],
        temp: i16,
        catalog: &Catalog,
    ) -> Result<(), String> {
        if !self.begin(operands) {
            return Ok(());
        }
        if operands[3] == 7 {
            return Err("portrait headline requires world resource selection".into());
        }
        self.apply_sprite(operands, temp, catalog)
    }
    fn apply_sprite(
        &mut self,
        operands: &[u8; 8],
        temp: i16,
        catalog: &Catalog,
    ) -> Result<(), String> {
        let options = u16::from_le_bytes(operands[6..8].try_into().unwrap());
        let index = i32::from(operands[2])
            + if options & 0x1000 != 0 {
                i32::from(temp)
            } else {
                0
            };
        let icon = sprite(operands[3], index);
        if icon == 0 {
            return Ok(());
        }
        let frames = catalog.frames.get(&icon).copied().unwrap_or(0) / 3;
        if frames == 0 {
            return Ok(());
        }
        self.sprite = icon;
        self.portrait = None;
        self.frames = i32::try_from(frames).map_err(|_| "headline frame count overflow")?;
        self.frame = if options & 0x400 != 0 {
            self.frames - 1
        } else {
            0
        };
        self.step = if options & 0x400 != 0 { -1 } else { 1 };
        self.finish(operands);
        Ok(())
    }
    fn apply_portrait(&mut self, operands: &[u8; 8], resource: Option<u32>) {
        self.sprite = u16::MAX;
        self.portrait = resource;
        self.frames = 1;
        self.frame = 0;
        self.step = 1; // 46dfe9 ignores the sprite reverse-animation option.
        self.finish(operands);
    }
    fn finish(&mut self, operands: &[u8; 8]) {
        let duration = i16::from_le_bytes([operands[4], operands[5]]);
        let options = u16::from_le_bytes([operands[6], operands[7]]);
        self.background = if operands[6] <= 2 {
            100 + u16::from(operands[6])
        } else {
            0
        };
        self.overlay = if options & 0x200 != 0 { 103 } else { 0 };
        self.flag = options & 0x100 != 0;
        self.duration = if duration == -1 || options & 0x800 == 0 {
            i32::from(duration)
        } else {
            self.frames
                .wrapping_mul(i32::from(duration))
                .wrapping_mul(12)
        };
        self.ticks = 0;
    }
    pub(crate) fn validate_resource(&self, program: &Program) -> Result<(), String> {
        let valid = if self.sprite == u16::MAX {
            self.frames == 1
                && self.frame == 0
                && self.step == 1
                && self.portrait.is_none_or(|guid| {
                    program
                        .neighbor_resource_flags
                        .as_ref()
                        .is_some_and(|r| r.contains_key(&guid))
                        || program.modules.values().any(|m| {
                            m.object_definition_words
                                .values()
                                .any(|w| w.len() > 15 && definition_guid(w) == guid)
                        })
                })
        } else if self.portrait.is_some() {
            false
        } else if self.sprite == 0 {
            self.duration == 0
        } else {
            let catalog = program
                .modules
                .get(&program.globals)
                .and_then(|m| m.headline_catalog.as_ref())
                .ok_or("saved headline has no sprite catalog")?;
            catalog.frames.get(&self.sprite).copied().unwrap_or(0) / 3 == self.frames as u32
                && self.frames > 0
                && (0..self.frames).contains(&self.frame)
                && [-1, 1].contains(&self.step)
        };
        if valid {
            Ok(())
        } else {
            Err("invalid saved headline resource".into())
        }
    }
    /// 0x46e004, called once after a completed object VM visit, even headlessly.
    pub fn advance(&mut self) -> Result<bool, String> {
        if self.duration > 0 {
            self.duration -= 1;
        }
        if self.duration != 0 {
            self.ticks = self.ticks.wrapping_add(1);
            if self.ticks == 12 {
                if self.frames <= 0 {
                    return Err("active headline has no animation frames".into());
                }
                self.frame = self.frame.wrapping_add(self.step).rem_euclid(self.frames);
                self.ticks = 0;
                return Ok(true);
            }
        }
        Ok(false)
    }
}
impl Vm {
    pub(crate) fn set_headline(
        &mut self,
        p: &Program,
        t: &Thread,
        b: &[u8; 8],
    ) -> Result<(bool, String), String> {
        let f = t.stack.last().ok_or("headline frame missing")?;
        let target = if b[0] & 1 != 0 {
            f.stack_object
        } else {
            f.caller
        };
        let object = self
            .world
            .objects
            .get_mut(&target)
            .ok_or("headline target missing (native error 0x15)")?;
        if let Some(state) = &mut object.headline
            && state.begin(b)
        {
            if b[3] == 7 {
                if let Some(resource) = portrait_resource(p, &self.world, t, b)? {
                    self.world
                        .objects
                        .get_mut(&target)
                        .unwrap()
                        .headline
                        .as_mut()
                        .unwrap()
                        .apply_portrait(b, resource);
                }
            } else {
                let temp = self.world.objects[&f.caller].temps[0];
                let catalog = p
                    .modules
                    .get(&p.globals)
                    .and_then(|m| m.headline_catalog.as_ref())
                    .ok_or("headline sprite catalog unavailable")?;
                self.world
                    .objects
                    .get_mut(&target)
                    .unwrap()
                    .headline
                    .as_mut()
                    .unwrap()
                    .apply_sprite(b, temp, catalog)?;
            }
        }
        Ok((
            true,
            format!("updated original headline state on #{target}"),
        ))
    }
    pub fn update_headline(&mut self, actor: crate::ObjectId) -> Result<(), String> {
        if let Some(state) = self
            .world
            .objects
            .get_mut(&actor)
            .and_then(|o| o.headline.as_mut())
        {
            if state.last_update != Some(self.tick) {
                state.advance()?;
                state.last_update = Some(self.tick);
            }
        }
        Ok(())
    }
}

fn definition_guid(words: &[i16]) -> u32 {
    u32::from(words[14] as u16) | (u32::from(words[15] as u16) << 16)
}

/// 45728d -> 43ae0a. The resource cache is a memoized immutable selection:
/// first catalog master in original file order, or self for ungrouped objects.
/// A missing group master is a null result, not a failed live-object lookup.
fn catalog_resource(p: &Program, definition: &crate::Definition) -> Result<Option<u32>, String> {
    let words = crate::construction::definition_words(p, definition)?;
    if words[10] == 0 {
        return Ok(Some(definition.guid));
    }
    let module = &p.modules[&definition.source];
    for id in &module.object_definition_order {
        let candidate = module
            .object_definition_words
            .get(id)
            .ok_or("headline resource order references missing definition")?;
        if candidate.len() <= 15 {
            return Err("headline catalog definition is incomplete".into());
        }
        if candidate[10] == words[10] && candidate[11] == -1 {
            return Ok(Some(definition_guid(candidate)));
        }
    }
    Ok(None)
}

/// Outer None means keep the previous headline; Some(None) installs a null
/// portrait resource. The raw selector ignores the temporary-index option.
fn portrait_resource(
    p: &Program,
    world: &crate::World,
    t: &Thread,
    b: &[u8; 8],
) -> Result<Option<Option<u32>>, String> {
    let f = t.stack.last().ok_or("headline frame missing")?;
    let live = match b[2] {
        0 => f.stack_object,
        1 => {
            let people = world
                .person
                .as_ref()
                .ok_or("headline neighborhood unavailable")?;
            let id = if let Some(slots) = &people.neighbor_slots {
                usize::try_from(f.stack_object)
                    .ok()
                    .and_then(|i| i.checked_sub(1))
                    .and_then(|i| slots.get(i))
                    .copied()
                    .flatten()
            } else {
                (f.stack_object > 0 && people.neighbors.contains_key(&f.stack_object))
                    .then_some(f.stack_object)
            };
            let Some(id) = id else {
                return Ok(None);
            };
            let neighbor = people
                .neighbors
                .get(&id)
                .ok_or("headline neighbor slot record absent")?;
            let resources = p
                .neighbor_resource_flags
                .as_ref()
                .ok_or("headline neighbor resource catalog unavailable")?;
            return Ok(resources
                .contains_key(&neighbor.guid)
                .then_some(Some(neighbor.guid)));
        }
        2 => {
            let index = i16::from_le_bytes([b[0], b[1]]) >> 1;
            if i32::from(index) >= f.locals.len() as i32 {
                return Err("headline local index invalid (native error 0x33)".into());
            }
            // 47518f returns the LOCAL base after arguments. Negative indices
            // can alias arguments/header/earlier frames; retain opaque pointers
            // as an explicit boundary rather than inventing an object ID.
            crate::raw_frame_word(
                t,
                f.storage_offset as i64 + 8 + f.args.len() as i64 + i64::from(index),
            )?
        }
        _ => return Ok(None),
    };
    let Some(object) = world.objects.get(&live).filter(|_| live > 0) else {
        return Ok(None);
    };
    let definition = object
        .definition
        .as_ref()
        .ok_or("headline live object resource unavailable")?;
    Ok(Some(catalog_resource(p, definition)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "private-fixtures")]
    #[test]
    fn retained_original_catalog_resource_selection_vectors() {
        let report: serde_json::Value =
            serde_json::from_str(include_str!("../tests/fixtures/multitile.json")).unwrap();
        for row in report["resources"].as_array().unwrap() {
            let case = &row["input"];
            let mut module = crate::Module {
                source: "file0".into(),
                ..Default::default()
            };
            for (index, def) in case["definitions"].as_array().unwrap().iter().enumerate() {
                if def["file"] != 0 {
                    continue;
                }
                let id = index as u16 + 1;
                let mut words = vec![0; 108];
                words[10] = def["group"].as_i64().unwrap() as i16;
                words[11] = def["index"].as_i64().unwrap() as i16;
                words[14] = id as i16 + 100;
                module.object_definition_words.insert(id, words);
                module.object_definition_order.push(id);
            }
            let object = case["part"].as_u64().unwrap() as u16;
            let program = Program {
                modules: BTreeMap::from([("file0".into(), module)]),
                ..Default::default()
            };
            let actual = catalog_resource(
                &program,
                &crate::Definition {
                    source: "file0".into(),
                    object,
                    guid: u32::from(object) + 100,
                    slots: 0,
                },
            )
            .unwrap();
            let original = row["original"]["catalog"].as_u64().unwrap() as u32;
            assert_eq!(actual, (original != 0).then_some(original + 100), "{case}");
        }
    }
}
