//! Read-only replacement Buy catalog over the actual mounted definitions.
//! Native membership, text precedence, signed prices, and bytewise comparison
//! are retained. Equal price/name products keep registration order in this UI;
//! the legacy mixed Build/Buy vector's introsort tie permutation is not copied.
use super::Catalog;
pub mod labels;
use serde::{Deserialize, Serialize};
use sims_formats::{behavior::InteractionTable, strings::StringTable};
use sims_sim::object_catalog::{
    move_tool::selected::lookup_master,
    products::{Definition, Eligibility, Masks},
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Product {
    /// One-based registered definition identity; separate from GUID and Sim ID.
    pub definition: u32,
    pub source: sims_vm::Definition,
    pub fields: Definition,
    pub name: Vec<u8>,
    pub description: Vec<u8>,
    pub language: u8,
}
impl Product {
    pub fn visible(&self, catalog: i32, masks: Masks) -> bool {
        self.fields.present(catalog) && self.fields.matches(catalog, masks)
    }
    pub fn display_name(&self) -> String {
        sims_formats::strings::decode(&self.name, self.language)
    }
    pub fn display_description(&self) -> String {
        sims_formats::strings::decode(&self.description, self.language)
    }
}

/// 4061e3's non-person branch followed by 457d3b's fallback. CATS strings
/// are consecutive NUL-terminated byte strings, not a StringTable header.
fn text(
    cats: Option<&[u8]>,
    ctss: Option<&[u8]>,
    fallback: &[u8],
    language: u8,
) -> Result<(Vec<u8>, Vec<u8>), String> {
    fn cstr(bytes: &[u8]) -> Result<(&[u8], &[u8]), String> {
        let n = bytes
            .iter()
            .position(|&b| b == 0)
            .ok_or("unterminated catalog string")?;
        Ok((&bytes[..n], &bytes[n + 1..]))
    }
    let (mut name, description) = if let Some(bytes) = cats {
        let (name, tail) = cstr(bytes)?;
        let description = if tail.is_empty() {
            &[][..]
        } else {
            cstr(tail)?.0
        };
        (name.to_vec(), description.to_vec())
    } else if let Some(bytes) = ctss {
        let table = StringTable::parse(bytes).map_err(|e| e.to_string())?;
        let get = |index| {
            table
                .entries
                .iter()
                .filter(|e| e.language == language)
                .nth(index)
                .or_else(|| table.entries.iter().filter(|e| e.language == 1).nth(index))
                // 4061e3 passes each retrieved string through5d4cf9's
                // C-string copy, including counted legacy STR encodings.
                .map(|e| e.raw_text.iter().copied().take_while(|&b| b != 0).collect())
                .unwrap_or_default()
        };
        (get(0), get(1))
    } else {
        (vec![], vec![])
    };
    if name.is_empty() {
        name.push(b'*');
        name.extend(fallback.iter().copied().take_while(|&b| b != 0));
    }
    Ok((name, description))
}

impl Catalog<'_> {
    /// 457cb0 tries the definition's local resource, then its semiglobal; a
    /// successfully loaded empty table shadows the fallback. It does not read
    /// a global TTAB or consider whether an interaction passes its test tree.
    fn product_has_interactions(&mut self, id: u32, fields: Definition) -> Result<bool, String> {
        let table = match fields.interactions {
            -1 => {
                let master = lookup_master(&self.definitions, id)?
                    .ok_or("catalog interaction table has no master")?;
                Definition::parse(&self.definitions.entries[master as usize - 1].data)?.interactions
            }
            n if n < 0 => 0,
            n => n,
        };
        if table <= 0 {
            return Ok(false);
        }
        let e = &self.definitions.entries[id as usize - 1];
        let resources = [Some(e.key.resource), e.semiglobal];
        for resource in resources.into_iter().flatten() {
            let chunk =
                self.resources
                    .by_id(resource, u32::from_be_bytes(*b"TTAB"), table as u32, 0)?;
            if chunk == 0 {
                continue;
            }
            let result = InteractionTable::parse(self.resources.data(chunk)?)
                .map(|t| !t.interactions.is_empty())
                .map_err(|e| e.to_string());
            self.resources.release_chunk(resource, chunk)?;
            return result;
        }
        Ok(false)
    }
    pub fn buy_products(&mut self, language: u8) -> Result<Vec<Product>, String> {
        self.ready()?;
        let mut products = Vec::new();
        for row in 0..self.definitions.entries.len() {
            let id = u32::try_from(row)
                .ok()
                .and_then(|n| n.checked_add(1))
                .ok_or("Buy definition identity overflow")?;
            let fields = Definition::parse(&self.definitions.entries[row].data)?;
            match fields.eligibility() {
                Eligibility::Excluded => continue,
                Eligibility::Interactions if !self.product_has_interactions(id, fields)? => {
                    continue;
                }
                _ => (),
            }
            let cats = self.portrait_chunk(id, *b"CATS", fields.strings)?;
            let ctss = if cats.is_none() {
                self.portrait_chunk(id, *b"CTSS", fields.strings)?
            } else {
                None
            };
            let (name, description) = text(
                cats.as_deref(),
                ctss.as_deref(),
                &self.definitions.entries[row].name,
                language,
            )
            .map_err(|e| format!("Buy definition {id}: {e}"))?;
            products.push(Product {
                definition: id,
                source: self.portrait_definition(id)?,
                fields,
                name,
                description,
                language,
            });
        }
        products.sort_by(|a, b| {
            a.fields
                .price
                .cmp(&b.fields.price)
                .then(a.name.cmp(&b.name))
        });
        Ok(products)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn legacy_text_wins_even_when_empty_and_preserves_encoded_bytes() {
        assert_eq!(
            text(
                Some(b"\x80 chair\0Description\0"),
                Some(b"invalid CTSS"),
                b"fallback",
                1
            )
            .unwrap(),
            (b"\x80 chair".to_vec(), b"Description".to_vec())
        );
        assert_eq!(
            text(
                Some(b"\0Old description\0"),
                Some(b"ignored"),
                b"Name\0tail",
                1
            )
            .unwrap(),
            (b"*Name".to_vec(), b"Old description".to_vec())
        );
        assert_eq!(
            text(None, None, b"Missing", 1).unwrap(),
            (b"*Missing".to_vec(), vec![])
        );
        assert!(text(Some(b"unterminated"), None, b"", 1).is_err());
    }
    #[test]
    fn ctss_uses_requested_language_with_original_byte_fallback() {
        let table = b"\xfd\xff\x03\x00\x01English\0\0\x01Description\0\0\x02Fran\xe7ais\0\0";
        assert_eq!(
            text(None, Some(table), b"Unused", 2).unwrap(),
            (b"Fran\xe7ais".to_vec(), b"Description".to_vec())
        );
        assert_eq!(
            text(None, Some(table), b"Unused", 1).unwrap(),
            (b"English".to_vec(), b"Description".to_vec())
        );
        let counted = b"\0\0\x02\0\x03A\0B\x04Text";
        assert_eq!(
            text(None, Some(counted), b"Unused", 1).unwrap(),
            (b"A".to_vec(), b"Text".to_vec())
        );
    }
}
