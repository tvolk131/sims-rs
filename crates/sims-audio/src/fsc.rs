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
//!    `NONE` for an empty cell, then any number of exclusion cells, each
//!    written `(a,b)`.
//!
//! The field names are the file's own; [`crate::ambience`] says what the
//! player does with them. *(code)* The original reads version 5 and 6
//! files too (17 cell numbers, no `stereo?`); every score in the install
//! is version 7, the only one read here. Sample names are bare file
//! names, opened in the directory the FSC file is in.

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

/// One cell: its 18 numbers, in the order of the file's cell header, its
/// sample file and its exclusion cells.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cell {
    pub fields: Vec<i64>,
    pub file: String,
    /// The `(a,b)` pairs after the file name, in order. *(code)* The
    /// player takes `a` as the column and `b` as the row. No score in the
    /// install has any.
    pub exclusions: Vec<(i64, i64)>,
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
            let rest = parts[20..].join("\t");
            let exclusions = exclusions(&rest).ok_or_else(|| {
                Error::text("FSC", line + 1, "an exclusion cell is not two numbers")
            })?;
            cells.push(Some(Cell {
                fields,
                file: file.to_owned(),
                exclusions,
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

/// The `(a,b)` pairs in the text after a cell's file name; `None` when one
/// is not two numbers. Text outside parentheses is ignored. (The original
/// splits the line at each `(` and `)`; what it does with text between
/// pairs, or a pair that is not two numbers, was not followed, as no
/// score in the install has an exclusion.)
fn exclusions(text: &str) -> Option<Vec<(i64, i64)>> {
    let mut pairs = Vec::new();
    let mut rest = text;
    while let Some(open) = rest.find('(') {
        let after = &rest[open + 1..];
        let close = after.find(')')?;
        let (a, b) = after[..close].split_once(',')?;
        pairs.push((number(a.trim())?, number(b.trim())?));
        rest = &after[close + 1..];
    }
    Some(pairs)
}
