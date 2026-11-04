//! Original UIText STR#150,200..207,210 used by5050e4/506765.
use serde::{Deserialize, Serialize};
use sims_formats::{iff::Iff, strings::StringTable};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Labels {
    pub source: String,
    pub language: u8,
    roots: [[Vec<u8>; 8]; 7],
    functions: [[Vec<u8>; 4]; 8],
    special: [Vec<u8>; 4],
}
impl Labels {
    pub fn load(index: &crate::Index, language: u8) -> anyhow::Result<Self> {
        use anyhow::Context;
        let resource = &index.resources[index.find("UIText.iff")?];
        let iff = Iff::parse(resource.read()?)?;
        let table = |id| -> anyhow::Result<StringTable> {
            Ok(StringTable::parse(iff.data(
                iff.get(b"STR#", id).context("Buy category text absent")?,
            ))?)
        };
        let get = |table: &StringTable, n: usize| {
            table
                .entries
                .iter()
                .filter(|e| e.language == language)
                .nth(n)
                .or_else(|| table.entries.iter().filter(|e| e.language == 1).nth(n))
                .map(|e| e.raw_text.iter().copied().take_while(|&b| b != 0).collect())
                .unwrap_or_default()
        };
        let roots = table(150)?;
        let special = table(210)?;
        let mut functions: [[Vec<u8>; 4]; 8] = Default::default();
        for (row, values) in functions.iter_mut().enumerate() {
            let t = table(200 + row as u16)?;
            *values = std::array::from_fn(|i| get(&t, i));
        }
        Ok(Self {
            source: resource.locator.clone(),
            language,
            roots: std::array::from_fn(|r| std::array::from_fn(|i| get(&roots, r * 8 + i))),
            functions,
            // Columns4,5,6,7 read210's one-based entries4,5,2,3.
            special: [3, 4, 1, 2].map(|i| get(&special, i)),
        })
    }
    pub fn root(&self, catalog: usize, row: usize) -> &[u8] {
        if catalog == 6 {
            match row {
                2 => &self.roots[5][4],
                3 => &self.roots[4][2],
                _ => &self.roots[5][row],
            }
        } else {
            &self.roots[catalog][row]
        }
    }
    pub fn category(&self, catalog: usize, subcatalog: i32, row: usize, column: usize) -> String {
        let bytes = if subcatalog == 0 {
            self.root(catalog, column)
        } else if catalog != 1 {
            &self.roots[1][column]
        } else if column < 4 {
            &self.functions[row][column]
        } else {
            &self.special[column - 4]
        };
        sims_formats::strings::decode(bytes, self.language)
    }
}
