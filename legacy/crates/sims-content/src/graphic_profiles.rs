//! Source-backed ordinary definition profile cache. One owner is shared by all
//! ordinary Animators using that definition. Mounted mutable StringSet resource
//! encoding is separate; short tables expose an explicit writeback callback.
use crate::Index;
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sims_formats::{iff::Iff, strings::StringTable};
use sims_sim::pose::graphic_profile::{self as native, Metadata, Reply, Request, Strings};
use sims_vm::Definition;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Profile {
    metadata: Metadata,
    strings: Vec<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cached {
    definition: Definition,
    state: native::State,
    update: native::Update,
    profile: Option<Profile>,
    source_hash: Option<[u8; 32]>,
}
impl Cached {
    pub fn new(definition: Definition, words: &[i16]) -> Result<Self> {
        Ok(Self {
            definition,
            state: native::State {
                kind: *words
                    .get(9)
                    .context("ordinary profile definition kind absent")?,
                id: *words.get(19).context("ordinary profile string id absent")? as u16,
                file: 1,
                cached: None,
            },
            update: native::Update::default(),
            profile: None,
            source_hash: None,
        })
    }
    pub fn definition(&self) -> &Definition {
        &self.definition
    }
    pub fn complete(&self) -> bool {
        self.update.complete()
    }
    pub fn strings(&self) -> Result<Option<&[String]>> {
        match self.update.result().map_err(anyhow::Error::msg)? {
            None => Ok(None),
            Some(1) => Ok(Some(
                &self
                    .profile
                    .as_ref()
                    .context("cached profile lost strings")?
                    .strings,
            )),
            _ => anyhow::bail!("source profile identity mismatch"),
        }
    }
    fn source<'a>(&self, index: &'a Index) -> Result<&'a crate::Resource> {
        let mut found = index
            .resources
            .iter()
            .filter(|r| r.locator == self.definition.source);
        let resource = found
            .next()
            .context("ordinary profile source not mounted")?;
        ensure!(found.next().is_none(), "ambiguous ordinary profile source");
        Ok(resource)
    }
    pub fn restore(index: &Index, saved: Self) -> Result<Self> {
        saved.update.validate().map_err(anyhow::Error::msg)?;
        ensure!(
            saved.state.file == 1 && saved.state.cached.is_none_or(|id| id == 1),
            "source profile identity mismatch"
        );
        ensure!(
            saved.profile.is_some() == saved.state.cached.is_some(),
            "source profile cache storage mismatch"
        );
        if let Some(hash) = saved.source_hash {
            ensure!(
                <[u8; 32]>::from(Sha256::digest(saved.source(index)?.read()?)) == hash,
                "ordinary profile source changed"
            );
        }
        Ok(saved)
    }
    /// Complete one cache service. Original files are always read-only. A
    /// table shorter than two entries requests real writeback through the host;
    /// returning Err keeps Load pending rather than silently acknowledging it.
    pub fn advance(
        &mut self,
        index: &Index,
        writeback: &mut impl FnMut(&Definition, u16, &[String]) -> Result<u32>,
    ) -> Result<bool> {
        let Some(request) = self
            .update
            .advance(&self.state)
            .map_err(anyhow::Error::msg)?
        else {
            return Ok(true);
        };
        let reply = match request {
            Request::Construct => {
                ensure!(self.profile.is_none(), "source profile constructed twice");
                self.profile = Some(Profile {
                    metadata: Metadata::default(),
                    strings: vec![],
                });
                Reply::Constructed(Some(1))
            }
            Request::Load { profile, file, id } => {
                ensure!(profile == 1 && file == 1);
                let bytes = self.source(index)?.read()?;
                let hash: [u8; 32] = Sha256::digest(&bytes).into();
                if let Some(previous) = self.source_hash {
                    ensure!(hash == previous, "ordinary profile source changed");
                }
                let iff = Iff::parse(bytes)?;
                let chunks = iff.resource_order(b"STR#")?;
                let mut found = chunks.into_iter().filter(|chunk| chunk.id == id);
                let chunk = found
                    .next()
                    .context("ordinary profile StringSet resource absent")?;
                ensure!(
                    found.next().is_none(),
                    "ambiguous ordinary profile StringSet resource"
                );
                let table = StringTable::parse(iff.data(chunk))?;
                // 455e97 explicitly selects language0 (disk language1), not
                // the user interface's current locale or a person fallback.
                let incoming = table
                    .entries
                    .into_iter()
                    .filter(|e| e.language == 1)
                    .map(|e| e.text)
                    .collect();
                self.source_hash = Some(hash);
                let loaded = self
                    .profile
                    .as_mut()
                    .context("ordinary profile not constructed")?;
                struct Source<'a, F> {
                    definition: &'a Definition,
                    incoming: Vec<String>,
                    strings: &'a mut Vec<String>,
                    writeback: &'a mut F,
                }
                impl<F: FnMut(&Definition, u16, &[String]) -> Result<u32>> Strings for Source<'_, F> {
                    fn read(&mut self, _: &Metadata) -> Result<u32, String> {
                        *self.strings = std::mem::take(&mut self.incoming);
                        Ok(1)
                    }
                    fn count(&mut self) -> Result<i32, String> {
                        i32::try_from(self.strings.len())
                            .map_err(|_| "ordinary profile count overflow".into())
                    }
                    fn append_empty(&mut self, index: i32) -> Result<(), String> {
                        if index != self.strings.len() as i32 + 1 {
                            return Err("ordinary profile append index mismatch".into());
                        }
                        self.strings.push(String::new());
                        Ok(())
                    }
                    fn write(&mut self, _: u64, id: u16) -> Result<u32, String> {
                        (self.writeback)(self.definition, id, self.strings)
                            .map_err(|e| e.to_string())
                    }
                }
                let result = native::load(
                    &mut loaded.metadata,
                    file,
                    id,
                    &mut Source {
                        definition: &self.definition,
                        incoming,
                        strings: &mut loaded.strings,
                        writeback,
                    },
                )
                .map_err(anyhow::Error::msg)?;
                Reply::Loaded(result)
            }
        };
        self.update
            .acknowledge(&mut self.state, &request, reply)
            .map_err(anyhow::Error::msg)?;
        Ok(self.complete())
    }
}
