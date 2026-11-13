//! Read an explicitly selected original zoning resource. No implicit installer
//! migration or writing into an original neighborhood is performed here.
use crate::Index;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sims_formats::{iff::Iff, strings::StringTable};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Zoning {
    pub source: String,
    pub sha256: String,
    pub rows: Vec<[i32; 2]>,
}
impl Zoning {
    pub fn load(index: &Index, source: &str, expansions: u16) -> Result<Self> {
        let resource = &index.resources[index.find(source)?];
        let bytes = resource.read()?;
        let sha256 = format!("{:x}", Sha256::digest(&bytes));
        let iff = Iff::parse(bytes)?;
        let chunk = iff.get(b"STR#", 1).context("zoning STR#1 absent")?;
        let strings = StringTable::parse(iff.data(chunk))?;
        let lines = strings
            .entries
            .into_iter()
            .filter(|e| e.language == 1)
            .map(|e| e.text)
            .collect::<Vec<_>>();
        let rows = sims_vm::zoning::parse(&lines, expansions).map_err(anyhow::Error::msg)?;
        Ok(Self {
            source: resource.locator.clone(),
            sha256,
            rows,
        })
    }
    pub fn query_table(&self) -> std::collections::BTreeMap<i16, i32> {
        sims_vm::zoning::query_table(&self.rows)
    }
}
