//! HSM files: the symbol table the HIT assembler wrote next to a program.
//!
//! Each line is `name value (0xvalue)`. The table mixes the instruction
//! names (`note_on 2`), the register names (`v1 5`, `patch 50`), constants
//! (`duckpri_low 10`), and the program's labels: `tkd_Name` is a track
//! data ID and `Name` the address it stands for. The file starts with the
//! word `note`, the first instruction name. The HOT file's `@name` fields
//! name these symbols.

use crate::hot::number;
use crate::{Error, Result};
use std::collections::BTreeMap;

/// One HSM file's symbols.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Symbols {
    /// Every line, in file order.
    pub lines: Vec<(String, i64)>,
    by_name: BTreeMap<String, i64>,
}

impl Symbols {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let text: String = bytes.iter().map(|&b| char::from(b)).collect();
        let mut symbols = Self::default();
        for (index, line) in text.split('\n').enumerate() {
            let fields: Vec<&str> = line.split_whitespace().collect();
            match fields.as_slice() {
                [] => {}
                [name, value, ..] => {
                    let Some(value) = number(value) else {
                        return Err(Error::text("HSM", index + 1, "value is not a number"));
                    };
                    symbols.insert(name, value);
                }
                _ => return Err(Error::text("HSM", index + 1, "line without a value")),
            }
        }
        Ok(symbols)
    }

    /// Adds a symbol. Lookups find the first of several with one name.
    pub fn insert(&mut self, name: &str, value: i64) {
        self.lines.push((name.to_owned(), value));
        self.by_name.entry(name.to_owned()).or_insert(value);
    }

    /// The value of a symbol. Names compare exactly.
    pub fn get(&self, name: &str) -> Option<i64> {
        self.by_name.get(name).copied()
    }
}
