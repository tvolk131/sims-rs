//! Original SPR2 labels supply the runtime floor category (49fd95/4a1463).
use super::*;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Binding {
    source: String,
    sha256: String,
    local: u16,
    category: u8,
}
pub fn load(index: &Index, house: &House) -> Result<BTreeMap<u16, Binding>> {
    let used: std::collections::BTreeSet<_> = house
        .floors
        .iter()
        .flat_map(|f| f.coverings.iter().copied())
        .filter(|id| *id != 0 && *id != 255)
        .collect();
    let mut result = BTreeMap::new();
    for id in used {
        let (name, local) = if (1..30).contains(&id) {
            ("floors.iff", id)
        } else {
            let mut matches = house.floor_map.iter().flat_map(|file| {
                file.entries
                    .iter()
                    .filter(move |e| e.id == id)
                    .map(move |e| (file.name.as_str(), e.local))
            });
            let binding = matches
                .next()
                .with_context(|| format!("footstep floor {id} has no resource mapping"))?;
            ensure!(
                matches.next().is_none(),
                "ambiguous saved footstep floor {id}"
            );
            binding
        };
        result.insert(id, read_binding(index, name, local)?);
    }
    Ok(result)
}

/// Current retained coverings need no House/starting-save reconstruction when
/// every used ID belongs to the original builtin range. Unknown custom IDs
/// require their actual mapping instead of an inferred filename.
pub fn from_builtin_coverings(
    index: &Index,
    ids: impl IntoIterator<Item = u16>,
) -> Result<BTreeMap<u16, Binding>> {
    let used: std::collections::BTreeSet<_> = ids
        .into_iter()
        .filter(|id| *id != 0 && *id != 255)
        .collect();
    let mut result = BTreeMap::new();
    for id in used {
        ensure!(
            (1..30).contains(&id),
            "retained footstep floor {id} requires its observed resource mapping"
        );
        result.insert(id, read_binding(index, "floors.iff", id)?);
    }
    Ok(result)
}

fn read_binding(index: &Index, name: &str, local: u16) -> Result<Binding> {
    let source = crate::surfaces::resource(index, name)?;
    let resource = &index.resources[source];
    let bytes = resource.read()?;
    let sha256 = format!("{:x}", Sha256::digest(&bytes));
    let iff = sims_formats::iff::Iff::parse(bytes)?;
    let chunk = iff
        .get(b"SPR2", local)
        .context("footstep floor sprite missing")?;
    let category = properties::surface::category(&chunk.label);
    Ok(Binding {
        source: resource.locator.clone(),
        sha256,
        local,
        category,
    })
}
pub fn restore(index: &Index, bindings: &BTreeMap<u16, Binding>) -> Result<()> {
    let mut sources = BTreeMap::new();
    for binding in bindings.values() {
        if !sources.contains_key(&binding.source) {
            let bytes = index.resources[index.find(&binding.source)?].read()?;
            let hash = format!("{:x}", Sha256::digest(&bytes));
            sources.insert(
                binding.source.clone(),
                (hash, sims_formats::iff::Iff::parse(bytes)?),
            );
        }
        let (hash, iff) = &sources[&binding.source];
        ensure!(*hash == binding.sha256, "footstep floor source changed");
        let label = &iff
            .get(b"SPR2", binding.local)
            .context("restored footstep floor missing")?
            .label;
        ensure!(
            properties::surface::category(label) == binding.category,
            "footstep floor category changed"
        );
    }
    Ok(())
}
pub fn categories(bindings: &BTreeMap<u16, Binding>) -> BTreeMap<u16, u8> {
    bindings.iter().map(|(&id, b)| (id, b.category)).collect()
}
