//! Global UI bitmap identities from the installed Res_*.h/RT tables.
//! 5a578d/5a3f89 select language then default; 4368fc..436930 crops `gz`
//! to the first quarter and sets the display-profile transparency key.
//! This reads the shipped declarative grammar, not arbitrary C headers.
use crate::{Index, Resource};
use anyhow::{Context, Result, ensure};
use std::{collections::BTreeMap, path::Path};

#[derive(Debug, Clone)]
pub struct Bitmaps {
    entries: BTreeMap<(i32, u8), String>,
    /// Keep unresolved installed identities explicit; never guess their IDs.
    unresolved: Vec<String>,
}

fn lines(text: &str) -> impl Iterator<Item = &str> {
    text.lines()
        .map(|line| line.split_once("//").map_or(line, |(line, _)| line).trim())
        .filter(|line| !line.is_empty())
}

fn constants(text: &str, values: &mut BTreeMap<String, i32>) -> Result<()> {
    for line in lines(text) {
        let line = line
            .strip_prefix("const Sint32 ")
            .context("unsupported UI constant declaration")?;
        let (name, value) = line
            .strip_suffix(';')
            .context("unterminated UI constant")?
            .split_once('=')
            .context("UI constant has no value")?;
        let name = name.trim();
        let value: i32 = value
            .trim()
            .parse()
            .context("UI constant is not a decimal integer")?;
        if let Some(old) = values.insert(name.into(), value) {
            ensure!(old == value, "conflicting UI constant {name}");
        }
    }
    Ok(())
}

fn table(
    text: &str,
    values: &BTreeMap<String, i32>,
    entries: &mut BTreeMap<(i32, u8), String>,
    unresolved: &mut Vec<String>,
) -> Result<()> {
    for line in lines(text) {
        let fields: Vec<_> = line
            .strip_prefix('{')
            .and_then(|s| s.strip_suffix('}'))
            .context("invalid UI resource row")?
            .split(',')
            .map(str::trim)
            .collect();
        if fields.first() == Some(&"INCLUDE") {
            ensure!(fields.len() == 2, "invalid UI include");
            // All four shipped headers are loaded before their three RT files.
            ensure!(
                matches!(
                    fields[1],
                    "Res_CPanel.h" | "Res_Nbhd.h" | "Res_Other.h" | "Res_Languages.h"
                ),
                "unsupported UI include"
            );
            continue;
        }
        ensure!(
            matches!(fields.first(), Some(&"BITMAP" | &"RLEBMP")) && matches!(fields.len(), 3 | 4),
            "unsupported UI resource row {line}"
        );
        let number = |name: &str| -> Result<i32> {
            values
                .get(name)
                .copied()
                .or_else(|| name.parse().ok())
                .with_context(|| format!("undefined UI constant {name}"))
        };
        let id = match number(fields[1]) {
            Ok(id) => id,
            Err(error) => {
                unresolved.push(error.to_string());
                continue;
            }
        };
        let language = if fields.len() == 4 {
            u8::try_from(number(fields[3])?)?
        } else {
            0
        };
        let path = fields[2]
            .strip_prefix('"')
            .and_then(|s| s.strip_suffix('"'))
            .context("unquoted UI resource path")?
            .replace('\\', "/");
        ensure!(
            !path.starts_with('/') && !path.split('/').any(|p| p == ".."),
            "invalid UI resource path"
        );
        ensure!(
            entries.insert((id, language), path).is_none(),
            "duplicate UI resource {id}/{language}"
        );
    }
    Ok(())
}

fn resource<'a>(index: &'a Index, path: &str) -> Result<Option<&'a Resource>> {
    let loose = format!("UIGraphics/{path}");
    // Resource-table files are explicitly opened below UIGraphics. Resolve
    // that exact installed path before an archived resource with the same name.
    let mut installed = index
        .resources
        .iter()
        .filter(|r| r.locator.replace('\\', "/").eq_ignore_ascii_case(&loose));
    if let Some(found) = installed.next() {
        ensure!(
            installed.next().is_none(),
            "ambiguous installed UI resource {path}"
        );
        return Ok(Some(found));
    }
    let mut found = index
        .resources
        .iter()
        .filter(|r| r.name.replace('\\', "/").eq_ignore_ascii_case(path));
    let first = found.next();
    ensure!(found.next().is_none(), "ambiguous UI resource {path}");
    Ok(first)
}

impl Bitmaps {
    pub fn load(index: &Index) -> Result<Self> {
        let mut values = BTreeMap::new();
        for name in [
            "Res_CPanel.h",
            "Res_Nbhd.h",
            "Res_Other.h",
            "Res_Languages.h",
        ] {
            let bytes = resource(index, name)?
                .with_context(|| format!("UI header absent: {name}"))?
                .read()?;
            constants(std::str::from_utf8(&bytes)?, &mut values).with_context(|| name)?;
        }
        let mut entries = BTreeMap::new();
        let mut unresolved = vec![];
        for name in ["Res_CPanel.RT", "Res_Nbhd.RT", "Res_Other.RT"] {
            let bytes = resource(index, name)?
                .with_context(|| format!("UI table absent: {name}"))?
                .read()?;
            table(
                std::str::from_utf8(&bytes)?,
                &values,
                &mut entries,
                &mut unresolved,
            )
            .with_context(|| name)?;
        }
        Ok(Self {
            entries,
            unresolved,
        })
    }

    pub fn image(
        &self,
        index: &Index,
        icon: sims_vm::dialog::GlobalBitmap,
        language: u8,
        key: [u8; 3],
    ) -> Result<Option<image::RgbaImage>> {
        let Some(path) = self
            .entries
            .get(&(icon.id, language))
            .or_else(|| self.entries.get(&(icon.id, 0)))
        else {
            ensure!(
                self.unresolved.is_empty(),
                "UI resource {} has no resolved mapping; installed table contains unresolved identities: {:?}",
                icon.id,
                self.unresolved
            );
            return Ok(None);
        };
        // A missing identity is a native no-image result. A mapped but absent or
        // malformed installed file is reported rather than hiding broken data.
        let bytes = resource(index, path)?
            .with_context(|| format!("mapped UI bitmap absent: {path}"))?
            .read()?;
        let format = match Path::new(path)
            .extension()
            .and_then(|p| p.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref()
        {
            Some("bmp") => image::ImageFormat::Bmp,
            Some("tga") => image::ImageFormat::Tga,
            _ => anyhow::bail!("unsupported mapped UI image format: {path}"),
        };
        let bitmap = image::load_from_memory_with_format(&bytes, format)?.to_rgba8();
        Self::cell(bitmap, icon.button_cell, key).map(Some)
    }

    fn cell(mut bitmap: image::RgbaImage, button: bool, key: [u8; 3]) -> Result<image::RgbaImage> {
        if button {
            let width = bitmap.width() / 4;
            ensure!(width > 0, "UI button atlas has no first cell");
            bitmap = image::imageops::crop_imm(&bitmap, 0, 0, width, bitmap.height()).to_image();
        }
        for pixel in bitmap.pixels_mut() {
            if pixel.0[..3] == key {
                pixel.0[3] = 0;
            }
        }
        Ok(bitmap)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "private original Complete Collection UI tables and bitmap assets"]
    fn installed_global_bitmaps_resolve_original_mood_button_and_full_image() -> Result<()> {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../game-data/cabinet-extracted");
        let index = Index::scan(&root)?;
        let bitmaps = Bitmaps::load(&index)?;
        ensure!(bitmaps.entries[&(4500, 0)] == "CPanel/Buttons/Mood.bmp");
        let button = bitmaps
            .image(
                &index,
                sims_vm::dialog::GlobalBitmap {
                    id: 4500,
                    button_cell: true,
                },
                1,
                [255, 0, 255],
            )?
            .context("Mood button absent")?;
        let full = bitmaps
            .image(
                &index,
                sims_vm::dialog::GlobalBitmap {
                    id: 4500,
                    button_cell: false,
                },
                1,
                [255, 0, 255],
            )?
            .context("Mood atlas absent")?;
        ensure!(button.width() == full.width() / 4 && button.height() == full.height());
        ensure!(
            button
                == image::imageops::crop_imm(&full, 0, 0, full.width() / 4, full.height())
                    .to_image()
        );
        ensure!(
            bitmaps
                .image(
                    &index,
                    sims_vm::dialog::GlobalBitmap {
                        id: i32::MAX,
                        button_cell: false
                    },
                    1,
                    [255, 0, 255]
                )
                .is_err()
        );
        let background = bitmaps
            .image(
                &index,
                sims_vm::dialog::GlobalBitmap {
                    id: 2037,
                    button_cell: false,
                },
                1,
                [255, 0, 255],
            )?
            .context("universal TGA absent")?;
        ensure!(background.dimensions() == (220, 183));
        ensure!(background.pixels().any(|p| p[3] == 0));
        ensure!(background.pixels().any(|p| p[3] == 255));
        ensure!(bitmaps.entries.len() == 634);
        ensure!(bitmaps.unresolved == ["undefined UI constant kStudiownBulldoze"]);
        eprintln!(
            "{} original UI resource/language entries, Mood atlas {:?}, first cell {:?}",
            bitmaps.entries.len(),
            full.dimensions(),
            button.dimensions()
        );
        Ok(())
    }
    #[test]
    fn installed_table_grammar_retains_language_and_rejects_ambiguous_ids() {
        let mut values = BTreeMap::new();
        constants(
            "// declarations\nconst Sint32 kMood = 4500;\nconst Sint32 kFrench = 3;",
            &mut values,
        )
        .unwrap();
        let mut entries = BTreeMap::new();
        let mut unresolved = vec![];
        table("{INCLUDE,Res_CPanel.h}\n{RLEBMP,kMood,\"CPanel/Mood.bmp\"}\n{BITMAP,kMood,\"CPanel/MoodFR.bmp\",kFrench}", &values, &mut entries, &mut unresolved).unwrap();
        assert_eq!(entries[&(4500, 0)], "CPanel/Mood.bmp");
        assert_eq!(entries[&(4500, 3)], "CPanel/MoodFR.bmp");
        assert!(
            table(
                "{BITMAP,kMood,\"other.bmp\"}",
                &values,
                &mut entries,
                &mut unresolved
            )
            .is_err()
        );
        assert!(
            table(
                "{BITMAP,kMissing,\"other.bmp\"}",
                &values,
                &mut BTreeMap::new(),
                &mut unresolved
            )
            .is_ok()
        );
        assert_eq!(unresolved, ["undefined UI constant kMissing"]);
    }
    #[test]
    fn gz_crops_first_quarter_gzi_keeps_atlas_and_both_key_transparency() {
        let mut image = image::RgbaImage::from_pixel(19, 3, image::Rgba([5, 10, 15, 255]));
        image.put_pixel(2, 1, image::Rgba([255, 0, 255, 255]));
        image.put_pixel(5, 1, image::Rgba([1, 2, 3, 255]));
        let cell = Bitmaps::cell(image.clone(), true, [255, 0, 255]).unwrap();
        assert_eq!(cell.dimensions(), (4, 3));
        assert_eq!(cell.get_pixel(2, 1).0, [255, 0, 255, 0]);
        let full = Bitmaps::cell(image, false, [255, 0, 255]).unwrap();
        assert_eq!(full.dimensions(), (19, 3));
        assert_eq!(full.get_pixel(5, 1).0, [1, 2, 3, 255]);
        assert_eq!(full.get_pixel(2, 1).0, [255, 0, 255, 0]);
    }
}
