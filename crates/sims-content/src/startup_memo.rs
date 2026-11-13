use crate::{
    Index,
    avatar_catalog::Directories,
    avatar_installation::{Installation, Loaded, Namespace, complete_collection_asset_bindings},
    startup_inputs::{MAX_BYTES, Session, Tape, Validation},
};
use anyhow::Result;
use sims_formats::avatar::{Skeleton, catalog::Asset, index::Kind};
use std::{collections::BTreeMap, sync::Arc};

mod appearance_discovery;

struct Entry {
    namespace: Namespace,
    tape: Tape,
    template: (Loaded, Directories),
}

pub(crate) struct Memo {
    index: Arc<Index>,
    entry: Option<Entry>,
    limit: usize,
    session: bool,
    snapshot: Option<(Loaded, Directories)>,
    skeletons: BTreeMap<String, (Skeleton, usize)>,
    appearances: Option<Arc<appearance_discovery::Discovery>>,
    animations: Option<Arc<crate::assets::NativeCatalog>>,
}

impl Memo {
    pub(crate) fn new(index: Arc<Index>) -> Self {
        Self {
            index,
            entry: None,
            limit: MAX_BYTES,
            session: false,
            snapshot: None,
            skeletons: BTreeMap::new(),
            appearances: None,
            animations: None,
        }
    }

    /// Live play treats installed assets as immutable until the session closes.
    /// Keep discovery results instead of rereading the entire installation for
    /// each temporary portrait/NPC animator. External restore still validates
    /// saved resource hashes and starts with a new memo.
    pub(crate) fn for_session(index: Arc<Index>) -> Self {
        Self {
            session: true,
            ..Self::new(index)
        }
    }

    /// Populate during loading, before the household becomes interactive.
    pub(crate) fn prepare_session(&mut self) -> Result<()> {
        if self.snapshot.is_none() {
            self.snapshot = Some(crate::clothing_catalog::installed_catalog(&self.index)?);
        }
        if self.appearances.is_none() {
            self.appearances = Some(Arc::new(appearance_discovery::Discovery::prepare(
                &self.index,
            )?));
        }
        if self.animations.is_none() {
            self.animations = Some(Arc::new(crate::assets::NativeCatalog::prepare(
                &self.index,
            )?));
        }
        Ok(())
    }

    pub(crate) fn fork_for_session(&self) -> Self {
        Self {
            snapshot: self
                .snapshot
                .clone()
                .or_else(|| self.entry.as_ref().map(|e| e.template.clone())),
            skeletons: self.skeletons.clone(),
            appearances: self.appearances.clone(),
            animations: self.animations.clone(),
            ..Self::for_session(self.index.clone())
        }
    }

    pub(crate) fn find_appearance_catalog(&mut self, index: &Index, name: &str) -> Result<usize> {
        if self.session && std::ptr::eq(self.index.as_ref(), index) {
            self.prepare_session()?;
            return self.appearances.as_ref().unwrap().find(index, name);
        }
        crate::avatar::find_appearance_catalog(index, name)
    }

    pub(crate) fn load_native_animations(
        &mut self,
        index: &Index,
        program: &mut sims_vm::Program,
        names: &std::collections::BTreeSet<String>,
    ) -> Result<crate::assets::Report> {
        if names
            .iter()
            .all(|name| program.animations.contains_key(name))
        {
            return Ok(crate::assets::Report::default());
        }
        if self.session && std::ptr::eq(self.index.as_ref(), index) {
            self.prepare_session()?;
            return self
                .animations
                .as_ref()
                .unwrap()
                .load(index, program, names);
        }
        crate::assets::load_named_native_animations(index, program, names)
    }

    pub(crate) fn session_catalog(&self, index: &Index) -> Option<(Loaded, Directories)> {
        (self.session && std::ptr::eq(self.index.as_ref(), index))
            .then(|| self.snapshot.clone())
            .flatten()
    }

    pub(crate) fn installed_catalog(&mut self, index: &Index) -> Result<(Loaded, Directories)> {
        if self.session && std::ptr::eq(self.index.as_ref(), index) {
            if self.snapshot.is_none() {
                self.snapshot = Some(crate::clothing_catalog::installed_catalog(index)?);
            }
            return Ok(self.snapshot.as_ref().unwrap().clone());
        }
        self.installed_catalog_using(index, |limit| Ok(Session::capture(limit)), Tape::validate)
    }

    pub(crate) fn load_skeleton(&mut self, index: &Index, name: &str) -> Result<(Skeleton, usize)> {
        let retained = self.session && std::ptr::eq(self.index.as_ref(), index);
        if retained && let Some(value) = self.skeletons.get(name) {
            return Ok(value.clone());
        }
        let (mut installed, directories) = self.installed_catalog(index)?;
        let record = installed
            .catalog
            .resolve(index, &directories, Kind::Skeleton, name.as_bytes())?
            .ok_or_else(|| {
                anyhow::anyhow!("skeleton {name} absent from installed avatar registry")
            })?;
        let Asset::Skeleton(skeleton) = &record.value else {
            anyhow::bail!("skeleton registry returned another asset kind");
        };
        let value = (skeleton.clone(), record.source);
        if retained {
            self.skeletons.insert(name.to_owned(), value.clone());
        }
        Ok(value)
    }

    fn installed_catalog_using(
        &mut self,
        index: &Index,
        capture: impl FnOnce(usize) -> Result<Session>,
        validate: impl FnOnce(&Tape, usize) -> Result<Validation>,
    ) -> Result<(Loaded, Directories)> {
        if !std::ptr::eq(self.index.as_ref(), index) {
            return crate::clothing_catalog::installed_catalog(index);
        }
        let installation =
            Installation::new(index, complete_collection_asset_bindings(&index.root))?;
        let namespace = installation.namespace();
        let inputs = match self.entry.take() {
            Some(entry) if entry.namespace == namespace => {
                match validate(&entry.tape, self.limit)? {
                    Validation::Unchanged => {
                        let result = entry.template.clone();
                        self.entry = Some(entry);
                        return Ok(result);
                    }
                    Validation::Changed(inputs) => inputs,
                }
            }
            _ => capture(self.limit)?,
        };
        let result =
            crate::clothing_catalog::installed_catalog_from(index, installation, Some(&inputs));
        let finished = inputs.finish();
        let result = result?;
        if let Some(tape) = finished? {
            self.entry = Some(Entry {
                namespace,
                tape,
                template: result.clone(),
            });
        }
        Ok(result)
    }
}

#[cfg(test)]
pub(crate) mod tests;

#[cfg(test)]
mod diagnostic;
