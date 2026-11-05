use crate::{
    Index,
    avatar_catalog::Directories,
    avatar_installation::{Installation, Loaded, Namespace, complete_collection_asset_bindings},
    startup_inputs::{MAX_BYTES, Session, Tape, Validation},
};
use anyhow::Result;
use std::sync::Arc;

struct Entry {
    namespace: Namespace,
    tape: Tape,
    template: (Loaded, Directories),
}

pub(crate) struct Memo {
    index: Arc<Index>,
    entry: Option<Entry>,
    limit: usize,
}

impl Memo {
    pub(crate) fn new(index: Arc<Index>) -> Self {
        Self {
            index,
            entry: None,
            limit: MAX_BYTES,
        }
    }

    pub(crate) fn installed_catalog(&mut self, index: &Index) -> Result<(Loaded, Directories)> {
        self.installed_catalog_using(index, |limit| Ok(Session::capture(limit)), Tape::validate)
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
