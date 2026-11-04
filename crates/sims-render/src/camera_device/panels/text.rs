//! Owned replacement text, shared by the original publication caller and Iced.
//! Native string references resolve to these fields; no Windows string objects
//! or surface caches are reconstructed.
use super::Owners;
use anyhow::{Result, bail, ensure};
use serde::{Deserialize, Serialize};
use sims_runtime::original_session::publication::PanelText;
use sims_vm::interface::view_control::{Query, Reply, Request, Text, Widget};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Texts {
    sources: PanelText,
    funds: Vec<u8>,
    clock: Vec<u8>,
    deferred_funds: Vec<u8>,
}
impl Texts {
    pub(super) fn property(
        &mut self,
        request: sims_vm::interface::property_refresh::Request,
        language: u8,
        bytes: Vec<u8>,
    ) -> Result<()> {
        ensure!(
            language == self.sources.language,
            "property text locale differs"
        );
        match request {
            sims_vm::interface::property_refresh::Request::Clock { .. } => {
                self.sources.clock = bytes
            }
            sims_vm::interface::property_refresh::Request::SetFunds(_) => {
                self.sources.funds = bytes
            }
            _ => bail!("non-text property callback"),
        }
        Ok(())
    }
    pub(super) fn new(sources: PanelText) -> Self {
        Self {
            sources,
            funds: Vec::new(),
            clock: Vec::new(),
            deferred_funds: Vec::new(),
        }
    }
    /// Source refresh happens at the actual frontend refresh, never on a
    /// Length/Equal callback. Resuming a pending frame retains its text.
    pub(super) fn refresh(&mut self, sources: PanelText, mask: u32) -> Result<()> {
        ensure!(
            sources.language == self.sources.language,
            "panel locale changed"
        );
        if mask & 0x4000 != 0 {
            self.sources.funds = sources.funds;
        }
        if mask & 0x2000 != 0 {
            self.sources.clock = sources.clock;
        }
        Ok(())
    }
    pub fn funds(&self) -> String {
        sims_formats::strings::decode(&self.funds, self.sources.language)
    }
    pub fn clock(&self) -> String {
        sims_formats::strings::decode(&self.clock, self.sources.language)
    }
    fn read(&self, owners: &Owners, reference: Text) -> Result<&[u8]> {
        let Text { owner, offset } = reference;
        Ok(if owner == owners.state && offset == 0x28 {
            &self.sources.funds
        } else if owner == owners.state && offset == 0x18 {
            &self.sources.clock
        } else if owner == owners.controls && offset == 0x194 {
            &self.deferred_funds
        } else if owner == owners.controls && offset == 0x170 {
            &self.clock
        } else if owner == owners.widgets[Widget::Status as usize] && offset == 0x4c {
            &self.funds
        } else {
            bail!("unbound panel text reference {reference:?}")
        })
    }
    pub(super) fn service(&mut self, owners: &Owners, request: Request) -> Result<Option<Reply>> {
        Ok(Some(match request {
            Request::Get {
                owner,
                query: query @ (Query::Status | Query::Title),
            } => {
                ensure!(owner == owners.state, "panel text source owner differs");
                Reply::Text(Text {
                    owner,
                    offset: if query == Query::Status { 0x28 } else { 0x18 },
                })
            }
            Request::Length(source) => Reply::Number(self.read(owners, source)?.len().try_into()?),
            Request::Equal { left, right } => Reply::Number(u32::from(
                self.read(owners, left)? == self.read(owners, right)?,
            )),
            Request::Copy { target, source } => {
                let value = self.read(owners, source)?.to_vec();
                ensure!(
                    target.owner == owners.controls,
                    "panel text destination owner differs"
                );
                match target.offset {
                    0x170 => self.clock = value,
                    0x194 => self.deferred_funds = value,
                    _ => bail!("unbound panel text destination {target:?}"),
                }
                Reply::Done
            }
            Request::SetText { widget, source } => {
                ensure!(
                    widget == owners.widgets[Widget::Status as usize],
                    "panel funds widget differs"
                );
                self.funds = self.read(owners, source)?.to_vec();
                Reply::Done
            }
            _ => return Ok(None),
        }))
    }
}
