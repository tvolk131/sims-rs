//! The interface's resource tables: `UIGraphics/Res_*.h` and `*.RT`.
//!
//! The original's resource manager (its own parser, named
//! `cGZResMgrRTParser` in the files' comments) reads each `.RT` table,
//! which maps a resource ID to a bitmap path under `UIGraphics`. The IDs
//! are named constants declared in the `.h` files the table includes. The
//! headers say no syntax other than line comments and the expected line
//! forms may be used, so only those forms are read:
//!
//! - header: `const Sint32 kName = 123;`
//! - table: `{INCLUDE,Res_CPanel.h}`, and `{BITMAP,kName,"Dir/File.bmp"}`
//!   or `{RLEBMP,...}`, with an optional fourth field naming a language.
//!
//! The old prototype read the same grammar
//! (legacy/crates/sims-content/src/dialog/ui_bitmaps.rs).

use crate::{Error, Result};
use std::collections::BTreeMap;

/// One bitmap row of a table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub id: i32,
    /// The path below `UIGraphics`, with `/` separators.
    pub path: String,
    /// The language the row is for: 0 for every language, 1 US English,
    /// and so on (`Res_Languages.h`).
    pub language: u8,
    /// `RLEBMP` rather than `BITMAP`: the original keeps the image
    /// run-length encoded in memory. It decodes to the same pixels.
    pub run_length: bool,
}

/// A parsed `.RT` table.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Table {
    /// The headers it includes, in order.
    pub includes: Vec<String>,
    pub rows: Vec<Row>,
    /// Rows whose ID names a constant no header declares, by that name.
    /// The install has one (`kStudiownBulldoze`); the original cannot
    /// resolve it either, so the row is kept out rather than guessed.
    pub unresolved: Vec<String>,
}

/// The text of a line with its `//` comment and surrounding space removed.
fn lines(text: &str) -> impl Iterator<Item = (usize, &str)> {
    text.lines()
        .enumerate()
        .map(|(n, line)| (n + 1, line.split_once("//").map_or(line, |(l, _)| l).trim()))
        .filter(|(_, line)| !line.is_empty())
}

fn fail<T>(line: usize, message: impl std::fmt::Display) -> Result<T> {
    Err(Error::new(0, format!("line {line}: {message}")))
}

/// Reads a header's constants into `constants`. A name declared twice
/// with different values is an error.
pub fn constants(text: &str, constants: &mut BTreeMap<String, i32>) -> Result<()> {
    for (n, line) in lines(text) {
        let Some(rest) = line.strip_prefix("const Sint32 ") else {
            return fail(n, "not a constant declaration");
        };
        let Some((name, value)) = rest.strip_suffix(';').and_then(|r| r.split_once('=')) else {
            return fail(n, "a constant needs `name = value;`");
        };
        let name = name.trim();
        let Ok(value) = value.trim().parse::<i32>() else {
            return fail(n, format!("{name} is not a decimal number"));
        };
        if let Some(old) = constants.insert(name.to_owned(), value)
            && old != value
        {
            return fail(n, format!("{name} is declared as both {old} and {value}"));
        }
    }
    Ok(())
}

/// Reads a table, resolving names with `constants`. The caller reads the
/// headers the table includes first.
pub fn table(text: &str, constants: &BTreeMap<String, i32>) -> Result<Table> {
    let mut table = Table::default();
    for (n, line) in lines(text) {
        let Some(inner) = line.strip_prefix('{').and_then(|l| l.strip_suffix('}')) else {
            return fail(n, "not a table row");
        };
        let fields: Vec<&str> = inner.split(',').map(str::trim).collect();
        match fields.as_slice() {
            ["INCLUDE", header] => table.includes.push((*header).to_owned()),
            [kind @ ("BITMAP" | "RLEBMP"), name, path, rest @ ..] if rest.len() <= 1 => {
                let number =
                    |name: &str| constants.get(name).copied().or_else(|| name.parse().ok());
                let Some(id) = number(name) else {
                    table.unresolved.push((*name).to_owned());
                    continue;
                };
                let language = match rest.first() {
                    None => 0,
                    Some(language) => match number(language).and_then(|l| u8::try_from(l).ok()) {
                        Some(l) => l,
                        None => return fail(n, format!("unknown language {language}")),
                    },
                };
                let Some(path) = path.strip_prefix('"').and_then(|p| p.strip_suffix('"')) else {
                    return fail(n, "the path is not quoted");
                };
                let path = path.replace('\\', "/");
                if path.is_empty() || path.starts_with('/') || path.split('/').any(|p| p == "..") {
                    return fail(n, format!("bad path {path}"));
                }
                table.rows.push(Row {
                    id,
                    path,
                    language,
                    run_length: *kind == "RLEBMP",
                });
            }
            _ => return fail(n, format!("unsupported row {line}")),
        }
    }
    Ok(table)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constants_and_rows_with_languages() {
        let mut values = BTreeMap::new();
        constants(
            "// header\nconst Sint32 kMood = 4500;\nconst Sint32 kTab\t\t=278;\n\nconst Sint32 kFrench = 3;",
            &mut values,
        )
        .unwrap();
        assert_eq!(values["kTab"], 278);
        let t = table(
            "{INCLUDE,Res_CPanel.h}\n{RLEBMP,kMood,\"CPanel\\Mood.bmp\"} // c\n\t\n{BITMAP,kMood,\"CPanel/MoodFR.bmp\",kFrench}\n{BITMAP,kGone,\"x.bmp\"}",
            &values,
        )
        .unwrap();
        assert_eq!(t.includes, ["Res_CPanel.h"]);
        assert_eq!(
            t.rows,
            [
                Row {
                    id: 4500,
                    path: "CPanel/Mood.bmp".into(),
                    language: 0,
                    run_length: true
                },
                Row {
                    id: 4500,
                    path: "CPanel/MoodFR.bmp".into(),
                    language: 3,
                    run_length: false
                }
            ]
        );
        assert_eq!(t.unresolved, ["kGone"]);
    }

    #[test]
    fn other_syntax_is_rejected() {
        let mut values = BTreeMap::new();
        assert!(constants("#define kMood 4500", &mut values).is_err());
        assert!(constants("const Sint32 kA = 1;\nconst Sint32 kA = 2;", &mut values).is_err());
        assert!(constants("const Sint32 kB = x;", &mut values).is_err());
        for bad in [
            "BITMAP,kA,\"a.bmp\"",
            "{SOUND,kA,\"a.wav\"}",
            "{BITMAP,1,a.bmp}",
            "{BITMAP,1,\"../a.bmp\"}",
            "{BITMAP,1,\"a.bmp\",kNoLanguage}",
            "{BITMAP,1,\"a.bmp\",1,2}",
        ] {
            assert!(table(bad, &values).is_err(), "{bad}");
        }
    }
}
