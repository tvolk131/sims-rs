//! Read-only price dependencies for the original house valuation pass.
//! IDs retain their imported FLRm/WALm bindings, rather than reallocating a
//! second catalog or assuming expansion directory order is precedence.
use crate::{Index, saves::House, surfaces};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sims_formats::{iff::Iff, strings::StringTable};
use sims_sim::{house_valuation::Prices, lot_geometry::FloorSource};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Catalog {
    pub prices: Prices,
    pub sources: BTreeMap<String, String>,
}
impl Catalog {
    pub fn validate(&self, index: &Index) -> Result<()> {
        for (source, hash) in &self.sources {
            let bytes = index.resources[index.find(source)?].read()?;
            ensure!(
                format!("{:x}", Sha256::digest(bytes)) == *hash,
                "valuation source changed: {source}"
            );
        }
        Ok(())
    }
}
fn strings(iff: &Iff, id: u16) -> Result<StringTable> {
    StringTable::parse(
        iff.data(
            iff.get(b"STR#", id)
                .context("valuation price StringSet absent")?,
        ),
    )
    .map_err(Into::into)
}
///636029 is the CRT atoi. Keep whitespace/sign, stop at the first nondigit
///and wrap through the same low32 bits, including empty/non-numeric strings.
pub fn atoi(text: &str) -> i32 {
    let mut bytes = text
        .as_bytes()
        .iter()
        .copied()
        // The original C-locale CRT also accepts vertical tab (0x0b), which
        // Rust's is_ascii_whitespace deliberately omits.
        .skip_while(|b| matches!(b, b' ' | b'\t'..=b'\r'))
        .peekable();
    let negative = matches!(bytes.peek(), Some(b'-'));
    if matches!(bytes.peek(), Some(b'-' | b'+')) {
        bytes.next();
    }
    let mut value = 0i32;
    for b in bytes.take_while(u8::is_ascii_digit) {
        value = value.wrapping_mul(10).wrapping_add(i32::from(b - b'0'));
    }
    if negative {
        value.wrapping_neg()
    } else {
        value
    }
}
fn number(table: &StringTable, index: usize) -> i32 {
    table.get(index, 1).map_or(0, atoi)
}

pub fn load(
    index: &Index,
    house: &House,
    size: usize,
    stride: usize,
    floors: &[FloorSource],
    pool: &[u8],
) -> Result<Catalog> {
    let mut sources = BTreeMap::new();
    let mut files = BTreeMap::<usize, Arc<Iff>>::new();
    let names = surfaces::ResourceNames::new(index);
    let mut file = |name: &str| -> Result<(String, Arc<Iff>)> {
        let id = names.resource(name)?;
        if !files.contains_key(&id) {
            let r = &index.resources[id];
            let bytes = r.read()?;
            sources.insert(r.locator.clone(), format!("{:x}", Sha256::digest(&bytes)));
            files.insert(id, Arc::new(Iff::parse(bytes)?));
        }
        Ok((index.resources[id].locator.clone(), files[&id].clone()))
    };
    let (_, build) = file("Build.iff")?;
    let wall_styles = strings(&build, 129)?;
    let mut prices = Prices {
        styles: vec![0; 256],
        pool: number(&strings(&build, 133)?, 0),
        floors: BTreeMap::new(),
        walls: BTreeMap::new(),
    };
    for (id, entry) in [(1, 0), (255, 0), (2, 3), (13, 6), (12, 9), (14, 12)] {
        prices.styles[id] = number(&wall_styles, entry);
    }
    let mut needed = BTreeSet::new();
    sims_sim::house_valuation::architecture(size, stride, floors, pool, &prices, |kind, id| {
        needed.insert((kind.to_owned(), id));
    })
    .map_err(anyhow::Error::msg)?;
    for (kind, id) in needed {
        let floor = kind == "floor";
        let builtin = if floor {
            (1..30).contains(&id)
        } else {
            id <= 30 || (248..=255).contains(&id)
        };
        let price = if builtin {
            if id == 0 || !floor && id >= 248 {
                0
            } else {
                number(
                    &strings(&build, if floor { 130 } else { 131 })?,
                    usize::from(id - 1) * 3,
                )
            }
        } else {
            let mappings = if floor {
                &house.floor_map
            } else {
                &house.wall_map
            };
            let mut bindings = mappings.iter().flat_map(|f| {
                f.entries
                    .iter()
                    .filter(move |e| e.id == id)
                    .map(move |e| (&f.name, e.local))
            });
            let (name, _local) = bindings.next().with_context(|| {
                format!("valuation {kind} {id} has no imported resource binding")
            })?;
            ensure!(
                bindings.next().is_none(),
                "duplicate valuation surface binding"
            );
            let (_, iff) = file(name)?;
            let fallback = number(&strings(&build, if floor { 134 } else { 135 })?, 0);
            if let Some(c) = iff.get(b"STR#", 0) {
                let t = StringTable::parse(iff.data(c))?;
                t.get(1, 1).map_or(fallback, atoi)
            } else {
                fallback
            }
        };
        if floor {
            prices.floors.insert(id, price);
        } else {
            prices.walls.insert(id, price);
        }
    }
    Ok(Catalog { prices, sources })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(feature = "private-fixtures")]
    #[test]
    fn native_integer_price_conversion_keeps_sign_prefix_and_wrapping() {
        let corpus: serde_json::Value =
            serde_json::from_str(include_str!("../tests/fixtures/native-price-atoi.json")).unwrap();
        assert_eq!(corpus["cases"].as_array().unwrap().len(), 11);
        for row in corpus["cases"].as_array().unwrap() {
            assert_eq!(
                atoi(row["input"].as_str().unwrap()),
                row["original"].as_i64().unwrap() as i32
            );
        }
    }
}
