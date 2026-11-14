//! FSC files: ambient sound scores (birds, wind, traffic outdoors).
//!
//! An FSC file is tab-separated text:
//!
//! 1. `version 7`;
//! 2. a comment row naming the score's fields (`# vol pri min max rows
//!    cols tempo BPB SelX SelY QuanX QuanY DifX DifY`);
//! 3. the score's values;
//! 4. a comment row naming the cell fields (`# vol rand? LR FB rand? fin
//!    fout dly rand? loop loop? quant prob pitchL pitchH fast? groupID
//!    stereo? filename exclusion cells`);
//! 5. `rows * cols` cell rows: 18 numbers and a sample file name, or
//!    `NONE` for an empty cell.
//!
//! The field names are the file's own; what the player does with them is
//! not read here. Sample names are bare file names, looked up in the
//! directory the FSC file is in. That the original resolves them this way
//! is **unverified**.

use crate::hot::number;
use crate::{Error, Result};

fn fail<T>(line: usize, message: impl Into<String>) -> Result<T> {
    Err(Error::text("FSC", line, message))
}

/// The score's values, in the order of the file's header row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Score {
    pub version: u32,
    /// `vol pri min max rows cols tempo BPB SelX SelY QuanX QuanY DifX DifY`.
    pub fields: Vec<i64>,
    pub rows: usize,
    pub columns: usize,
    /// `rows * columns` cells, row by row.
    pub cells: Vec<Option<Cell>>,
}

/// One cell: its 18 numbers, in the order of the file's cell header, and
/// its sample file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cell {
    pub fields: Vec<i64>,
    pub file: String,
}

impl Score {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let text: String = bytes.iter().map(|&b| char::from(b)).collect();
        let lines: Vec<&str> = text.split('\n').map(|l| l.trim_end_matches('\r')).collect();
        let version = lines
            .first()
            .and_then(|l| l.split_whitespace().collect::<Vec<_>>().get(1).copied())
            .filter(|_| lines[0].starts_with("version"))
            .and_then(number)
            .and_then(|n| u32::try_from(n).ok());
        let Some(version) = version else {
            return fail(1, "no version line");
        };
        let numbers = |line: usize| -> Result<Vec<i64>> {
            lines
                .get(line)
                .unwrap_or(&"")
                .split('\t')
                .map(str::trim)
                .filter(|f| !f.is_empty())
                .map(|f| {
                    number(f).ok_or_else(|| {
                        Error::text("FSC", line + 1, format!("{f:?} is not a number"))
                    })
                })
                .collect()
        };
        if !lines.get(1).is_some_and(|l| l.starts_with('#')) {
            return fail(2, "no score header row");
        }
        let fields = numbers(2)?;
        if fields.len() < 6 {
            return fail(3, "score row with fewer than six fields");
        }
        let (rows, columns) = (fields[4], fields[5]);
        let (Ok(rows), Ok(columns)) = (usize::try_from(rows), usize::try_from(columns)) else {
            return fail(3, "negative size");
        };
        let Some(count) = rows.checked_mul(columns).filter(|&n| n <= 65_536) else {
            return fail(3, "score too large");
        };
        if !lines.get(3).is_some_and(|l| l.starts_with('#')) {
            return fail(4, "no cell header row");
        }
        let mut cells = Vec::with_capacity(count);
        for index in 0..count {
            let line = 4 + index;
            let Some(text) = lines.get(line) else {
                return fail(line + 1, "fewer cells than rows times columns");
            };
            let parts: Vec<&str> = text.split('\t').map(str::trim).collect();
            // Field 0 is empty (each row starts with a tab); 1..=18 are
            // numbers; 19 is the file name.
            if parts.len() < 20 {
                return fail(line + 1, "cell with fewer than 19 fields");
            }
            let file = parts[19];
            if file.eq_ignore_ascii_case("NONE") {
                cells.push(None);
                continue;
            }
            let fields = parts[1..19]
                .iter()
                .map(|f| {
                    number(f).ok_or_else(|| {
                        Error::text("FSC", line + 1, format!("{f:?} is not a number"))
                    })
                })
                .collect::<Result<Vec<_>>>()?;
            cells.push(Some(Cell {
                fields,
                file: file.to_owned(),
            }));
        }
        Ok(Self {
            version,
            fields,
            rows,
            columns,
            cells,
        })
    }
}
