use crate::Index;
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const INSTALLED_HOUSES_ASCENDING_V1: &str = "installed-houses-ascending-v1";
pub const MAX_MATCHING_HOUSES: usize = 4096;
pub const MAX_LOCATOR_BYTES: usize = 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Candidate {
    lot: i16,
    locator: String,
}
impl Candidate {
    pub fn lot(&self) -> i16 {
        self.lot
    }
    pub fn locator(&self) -> &str {
        &self.locator
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstalledHouses {
    candidates: Vec<Candidate>,
    order: Vec<i32>,
}

fn lot_id(name: &str) -> Result<Option<i16>> {
    let lower = name.to_ascii_lowercase();
    let Some(digits) = lower
        .strip_prefix("house")
        .and_then(|name| name.strip_suffix(".iff"))
    else {
        return Ok(None);
    };
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return Ok(None);
    }
    let mut lot = 0i16;
    for digit in digits.bytes() {
        lot = lot
            .checked_mul(10)
            .and_then(|lot| lot.checked_add(i16::from(digit - b'0')))
            .context("installed house ID exceeds positive signed-short range")?;
    }
    Ok(Some(lot))
}

fn houses_directory(house: &str, neighborhood: &str, current: i16) -> Result<String> {
    ensure!(
        house.len() <= MAX_LOCATOR_BYTES && neighborhood.len() <= MAX_LOCATOR_BYTES,
        "installed house identity exceeds locator bound"
    );
    let (namespace, filename) = neighborhood
        .split_once('/')
        .context("installed house neighborhood namespace absent")?;
    let lower = namespace.to_ascii_lowercase();
    let suffix = lower
        .strip_prefix("userdata")
        .context("unsupported installed house neighborhood namespace")?;
    ensure!(
        filename.eq_ignore_ascii_case("Neighborhood.iff")
            && (suffix.is_empty()
                || (suffix.bytes().all(|byte| byte.is_ascii_digit())
                    && suffix.parse::<u16>().is_ok_and(|number| number > 0))),
        "invalid installed house neighborhood namespace"
    );
    let directory = format!("{namespace}/Houses/");
    let name = house
        .get(directory.len()..)
        .filter(|_| {
            house
                .get(..directory.len())
                .is_some_and(|prefix| prefix.eq_ignore_ascii_case(&directory))
        })
        .context("installed current house belongs to another namespace")?;
    ensure!(
        current > 0 && lot_id(name)? == Some(current),
        "installed current house filename/lot differs"
    );
    Ok(directory)
}

impl InstalledHouses {
    pub fn discover(index: &Index, house: &str, neighborhood: &str, current: i16) -> Result<Self> {
        let directory = houses_directory(house, neighborhood, current)?;
        let mut candidates = BTreeMap::new();
        for resource in &index.resources {
            if resource.archived || resource.locator.contains("::") {
                continue;
            }
            let Some(name) = resource.locator.get(directory.len()..).filter(|_| {
                resource
                    .locator
                    .get(..directory.len())
                    .is_some_and(|prefix| prefix.eq_ignore_ascii_case(&directory))
            }) else {
                continue;
            };
            let Some(lot) = lot_id(name)? else {
                continue;
            };
            ensure!(
                resource.locator.len() <= MAX_LOCATOR_BYTES,
                "installed house locator exceeds bound"
            );
            ensure!(
                candidates.len() < MAX_MATCHING_HOUSES,
                "installed house candidates exceed bound"
            );
            ensure!(
                candidates.insert(lot, resource.locator.clone()).is_none(),
                "duplicate or numeric alias for installed house {lot}"
            );
        }
        let result = Self {
            order: candidates
                .keys()
                .filter(|lot| **lot != 0 && **lot != current)
                .map(|lot| i32::from(*lot))
                .collect(),
            candidates: candidates
                .into_iter()
                .map(|(lot, locator)| Candidate { lot, locator })
                .collect(),
        };
        result.validate(house, neighborhood, current)?;
        Ok(result)
    }

    pub fn validate(&self, house: &str, neighborhood: &str, current: i16) -> Result<()> {
        let directory = houses_directory(house, neighborhood, current)?;
        ensure!(
            !self.order.is_empty()
                && self.candidates.len() <= MAX_MATCHING_HOUSES
                && self.order.len() <= MAX_MATCHING_HOUSES,
            "installed house replacement is empty or exceeds bound"
        );
        let mut previous = None;
        let mut current_found = false;
        let mut order = Vec::new();
        for candidate in &self.candidates {
            ensure!(
                candidate.locator.len() <= MAX_LOCATOR_BYTES
                    && candidate
                        .locator
                        .get(..directory.len())
                        .is_some_and(|prefix| prefix.eq_ignore_ascii_case(&directory))
                    && previous.is_none_or(|lot| candidate.lot > lot),
                "installed house manifest namespace/order/alias differs"
            );
            ensure!(
                lot_id(&candidate.locator[directory.len()..])? == Some(candidate.lot),
                "installed house manifest filename/lot differs"
            );
            if candidate.lot == current {
                ensure!(
                    candidate.locator.eq_ignore_ascii_case(house),
                    "installed current house manifest differs"
                );
                current_found = true;
            } else if candidate.lot != 0 {
                order.push(i32::from(candidate.lot));
            }
            previous = Some(candidate.lot);
        }
        ensure!(
            current_found,
            "installed current house absent from manifest"
        );
        ensure!(
            self.order == order,
            "installed house resolved order differs"
        );
        Ok(())
    }

    pub fn candidates(&self) -> &[Candidate] {
        &self.candidates
    }
    pub fn nearby_houses(&self) -> &[i32] {
        &self.order
    }
}
