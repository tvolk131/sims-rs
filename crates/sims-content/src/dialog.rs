//! Original job-popup atlas selected by 0x52f2fc (resource 4905), with native
//! 0x530a7b row selection. Decoding remains presentation data, outside the VM.
use crate::Index;
use anyhow::{Context, Result, ensure};
pub mod neighbor;
pub mod portrait;
pub mod ui_bitmaps;

/// CPanel resource4980/TutHigh.bmp, initialized as three horizontal frames by
/// the original UI. The pixel key is an explicit host display-profile input.
pub fn tutorial_highlight(index: &Index, key: [u8; 3]) -> Result<image::RgbaImage> {
    let mut matches = index.resources.iter().filter(|r| {
        r.name
            .replace('\\', "/")
            .eq_ignore_ascii_case("cpanel/tuthigh.bmp")
    });
    let resource = matches
        .next()
        .context("original tutorial highlight bitmap absent")?;
    ensure!(
        matches.next().is_none(),
        "ambiguous tutorial highlight bitmap"
    );
    let atlas =
        image::load_from_memory_with_format(&resource.read()?, image::ImageFormat::Bmp)?.to_rgba8();
    ensure!(
        atlas.width() == 150 && atlas.height() == 50,
        "unexpected original highlight atlas dimensions"
    );
    let mut frame = image::imageops::crop_imm(&atlas, 0, 0, 50, 50).to_image();
    for pixel in frame.pixels_mut() {
        if pixel.0[..3] == key {
            pixel.0[3] = 0;
        }
    }
    Ok(frame)
}

/// 0x43601c -> 0x5a5752 -> 0x4dece6 requests BMP_ from the frame resource.
/// An absent chunk produces no image in the original; decoding errors are
/// reported instead of silently replacing malformed content.
pub fn private_bitmap(index: &Index, source: &str, chunk: u16) -> Result<Option<image::RgbaImage>> {
    private_bitmap_language(index, source, chunk, 0)
}

/// IFFResFile2 0x419a5d -> 0x418e6c: requested nonzero language, then
/// default0, preserving the live resource enumeration's duplicate order.
/// The presentation profile supplies the language; no VM lookup is changed.
pub fn private_bitmap_language(
    index: &Index,
    source: &str,
    chunk: u16,
    language: u8,
) -> Result<Option<image::RgbaImage>> {
    let resource = &index.resources[index.find(source)?];
    let iff = sims_formats::iff::Iff::parse(resource.read()?)?;
    let chunks = iff.resource_order(b"BMP_")?;
    let Some(bitmap) = private_bitmap_chunk(&chunks, chunk, language) else {
        return Ok(None);
    };
    Ok(Some(
        image::load_from_memory_with_format(iff.data(bitmap), image::ImageFormat::Bmp)?.to_rgba8(),
    ))
}

fn private_bitmap_chunk<'a>(
    chunks: &[&'a sims_formats::iff::Chunk],
    id: u16,
    language: u8,
) -> Option<&'a sims_formats::iff::Chunk> {
    let find = |language| {
        chunks
            .iter()
            .copied()
            .find(|c| c.id == id && (c.flags >> 8) as u8 == language)
    };
    if language == 0 {
        find(0)
    } else {
        find(language).or_else(|| find(0))
    }
}

#[cfg(test)]
mod private_bitmap_tests {
    use super::*;
    #[cfg(feature = "private-fixtures")]
    #[test]
    fn language_and_duplicate_order_match_original_418e6c() -> Result<()> {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../tests/fixtures/tutorial-bitmap-language.json"
        ))?;
        for row in fixture["cases"].as_array().unwrap() {
            let input = &row["input"];
            let chunks = input["entries"]
                .as_array()
                .unwrap()
                .iter()
                .enumerate()
                .map(|(i, e)| sims_formats::iff::Chunk {
                    tag: *b"BMP_",
                    id: e[0].as_u64().unwrap() as u16,
                    flags: e[1].as_u64().unwrap() as u16,
                    label: String::new(),
                    offset: i,
                    payload: 0..0,
                })
                .collect::<Vec<_>>();
            let found = private_bitmap_chunk(
                &chunks.iter().collect::<Vec<_>>(),
                input["id"].as_u64().unwrap() as u16,
                input["language"].as_u64().unwrap() as u8,
            );
            assert_eq!(
                found.map(|c| c.offset as u64),
                row["original"].as_u64(),
                "{input}"
            );
        }
        Ok(())
    }
}

#[derive(Debug)]
pub struct JobIcons {
    atlas: image::RgbaImage,
}
impl JobIcons {
    pub fn load(index: &Index) -> Result<Self> {
        let mut matches = index.resources.iter().filter(|r| {
            r.name
                .replace('\\', "/")
                .eq_ignore_ascii_case("cpanel/backgrounds/jobiconmultipopup.bmp")
        });
        let resource = matches
            .next()
            .context("original job-popup bitmap missing")?;
        ensure!(
            matches.next().is_none(),
            "ambiguous original job-popup bitmap"
        );
        let atlas =
            image::load_from_memory_with_format(&resource.read()?, image::ImageFormat::Bmp)?
                .to_rgba8();
        ensure!(
            atlas.width() > 0 && atlas.height() > 0 && atlas.height() % 22 == 0,
            "invalid 22-row original job-popup atlas"
        );
        Ok(Self { atlas })
    }
    pub fn icon(&self, job: i32) -> Option<image::RgbaImage> {
        let [left, top, right, bottom] = sims_vm::dialog::job_icon_rect(
            job,
            [0, 0, self.atlas.width() as i32, self.atlas.height() as i32],
        )?;
        Some(
            image::imageops::crop_imm(
                &self.atlas,
                left as u32,
                top as u32,
                (right - left) as u32,
                (bottom - top) as u32,
            )
            .to_image(),
        )
    }
}
