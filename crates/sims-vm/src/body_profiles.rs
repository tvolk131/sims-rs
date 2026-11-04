//! Definition-owned BodyStrings caches (457f60) and published STR# overlays.
//! Two definitions may share one bank without sharing an already-loaded cache.
//! Publications only affect subsequent loads; original Program data is immutable.
use crate::{Definition, Program};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Cached {
    bank: u16,
    values: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    definitions: BTreeMap<String, BTreeMap<u16, Cached>>,
    banks: BTreeMap<String, BTreeMap<u16, Vec<String>>>,
}

fn bank(program: &Program, source: &str, object: u16) -> Result<Option<u16>, String> {
    let words = program
        .modules
        .get(source)
        .and_then(|m| m.object_definition_words.get(&object))
        .ok_or("body profile definition absent")?;
    if *words.get(9).ok_or("body profile object kind absent")? != 2 {
        return Ok(None);
    }
    Ok(Some(
        *words.get(19).ok_or("body profile bank ID absent")? as u16
    ))
}

impl State {
    /// 457f60(0): the caller may inspect this cache repeatedly without reloading
    /// newer published bank contents. The modifying flag-1 branch is separate.
    pub fn load(&mut self, program: &Program, d: &Definition) -> Result<Option<&[String]>, String> {
        let Some(id) = bank(program, &d.source, d.object)? else {
            return Ok(None);
        };
        if !self
            .definitions
            .get(&d.source)
            .is_some_and(|m| m.contains_key(&d.object))
        {
            let mut values = self
                .banks
                .get(&d.source)
                .and_then(|m| m.get(&id))
                .or_else(|| {
                    program
                        .modules
                        .get(&d.source)
                        .and_then(|m| m.strings.get(&id))
                })
                .ok_or("body profile source strings absent")?
                .clone();
            // 587b4b publishes padding only when it had to append entries.
            if values.len() < 35 {
                values.resize(35, String::new());
                self.banks
                    .entry(d.source.clone())
                    .or_default()
                    .insert(id, values.clone());
            }
            self.definitions
                .entry(d.source.clone())
                .or_default()
                .insert(d.object, Cached { bank: id, values });
        }
        Ok(Some(self.get(d)?))
    }

    pub fn get(&self, d: &Definition) -> Result<&[String], String> {
        Ok(&self
            .definitions
            .get(&d.source)
            .and_then(|m| m.get(&d.object))
            .ok_or("body profile cache not loaded")?
            .values)
    }

    /// Mutate an existing definition cache without implicitly publishing it.
    /// Callers own the enclosing service transaction and ordered writebacks.
    pub fn get_mut(&mut self, d: &Definition) -> Result<&mut [String], String> {
        Ok(&mut self
            .definitions
            .get_mut(&d.source)
            .and_then(|m| m.get_mut(&d.object))
            .ok_or("body profile cache not loaded")?
            .values)
    }

    pub fn publish(&mut self, d: &Definition) -> Result<(), String> {
        let cached = self
            .definitions
            .get(&d.source)
            .and_then(|m| m.get(&d.object))
            .ok_or("body profile cache not loaded")?;
        self.banks
            .entry(d.source.clone())
            .or_default()
            .insert(cached.bank, cached.values.clone());
        Ok(())
    }

    pub fn validate(&self, program: &Program) -> Result<(), String> {
        for (source, definitions) in &self.definitions {
            for (&object, cached) in definitions {
                if bank(program, source, object)? != Some(cached.bank) || cached.values.len() < 35 {
                    return Err("saved body profile cache layout invalid".into());
                }
            }
        }
        for (source, banks) in &self.banks {
            let module = program
                .modules
                .get(source)
                .ok_or("saved body profile module absent")?;
            for (&bank, values) in banks {
                if !module.strings.contains_key(&bank)
                    || values.len() < 35
                    || !self
                        .definitions
                        .get(source)
                        .is_some_and(|m| m.values().any(|c| c.bank == bank))
                {
                    return Err("saved body profile publication invalid".into());
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "body_profiles_tests.rs"]
mod tests;
