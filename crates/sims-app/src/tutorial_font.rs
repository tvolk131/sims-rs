//! Tutorial body font only:607d9e selects VariableSans_12.ffn;60b97f builds
//! cTSFont metrics. The Gimex4-bit glyph indices are validated against a native
//! atlas. Iced alpha blending remains a display-service approximation to565.
use anyhow::{Context, Result, ensure};
use iced::advanced::image::Renderer as _;
use iced::{Point, Rectangle, Size};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct Metric {
    pub packed: [i32; 4],
    pub bearing: [i32; 2],
    pub advance: i32,
}
impl Metric {
    pub fn ink(self) -> i32 {
        self.packed[2] - self.packed[0]
    }
}
#[derive(Debug, Clone)]
pub(super) struct Font {
    pub metrics: [Metric; 256],
    pub line_height: i32,
    pub width_addition: i32,
    pub decoded: sims_formats::ffn::Font,
    encoding: BTreeMap<char, u8>,
    images: [Option<iced::widget::image::Handle>; 256],
}
impl Font {
    pub(super) fn load(index: &sims_content::Index) -> Result<Self> {
        let path = "Fonts/variablesans_12.ffn";
        let loose = format!("UIGraphics/{path}");
        let installed = index
            .resources
            .iter()
            .filter(|r| r.locator.replace('\\', "/").eq_ignore_ascii_case(&loose))
            .collect::<Vec<_>>();
        let candidates = if installed.is_empty() {
            index
                .resources
                .iter()
                .filter(|r| r.name.replace('\\', "/").eq_ignore_ascii_case(path))
                .collect::<Vec<_>>()
        } else {
            installed
        };
        ensure!(
            candidates.len() == 1,
            "tutorial bitmap font absent/ambiguous"
        );
        Self::decode(&candidates[0].read()?)
    }
    fn decode(bytes: &[u8]) -> Result<Self> {
        let decoded = sims_formats::ffn::Font::parse(bytes)?;
        ensure!(
            decoded.flags == 9,
            "unsupported tutorial font palette profile"
        );
        let mut metrics = [Metric::default(); 256];
        let (mut row, mut maximum_height) = (0, 0);
        let mut previous: Option<[i32; 4]> = None;
        for g in &decoded.glyphs {
            let code = usize::from(g.code);
            ensure!(code < 256, "tutorial font is not single-byte");
            let (w, h) = (i32::from(g.width), i32::from(g.height));
            maximum_height = maximum_height.max(h);
            let mut packed = [0, row, w, row + h];
            if let Some(last) = previous {
                if last[1] == packed[1] {
                    packed[0] += last[2];
                    packed[2] += last[2];
                }
                if packed[2] > 1024 {
                    packed[2] -= packed[0];
                    packed[0] = 0;
                    packed[1] += maximum_height;
                    packed[3] += maximum_height;
                    row += maximum_height;
                }
            }
            metrics[code] = Metric {
                packed,
                bearing: [i32::from(g.bearing_x), i32::from(g.bearing_y)],
                advance: w + i32::from(g.bearing_x),
            };
            previous = Some(packed);
        }
        let (mut line_height, mut minimum_y, mut count, mut sum_width) = (0, 0, 0, 0);
        for (byte, m) in metrics.iter().enumerate() {
            if byte != 160 && m.packed[0] < m.packed[2] && m.packed[1] < m.packed[3] {
                line_height = line_height.max(m.packed[3] - m.packed[1] + m.bearing[1]);
                minimum_y = minimum_y.min(m.bearing[1]);
                count += 1;
                sum_width += m.ink();
            }
        }
        ensure!(count > 0, "tutorial font has no drawable glyphs");
        if minimum_y < 0 {
            line_height -= minimum_y;
            //60b97f's byteOffset<0x1be4 excludes byte255 from this adjustment.
            for m in &mut metrics[..255] {
                m.bearing[1] -= minimum_y;
            }
        }
        let width_addition = (decoded
            .glyphs
            .iter()
            .map(|g| f64::from(g.bearing_x))
            .sum::<f64>()
            / decoded.glyphs.len() as f64
            + 0.5)
            .trunc() as i32;
        metrics[32].packed = [0; 4];
        metrics[32].advance = (f64::from(sum_width) * 0.5 / f64::from(count) + 0.5).trunc() as i32;
        ensure!(
            line_height == 23,
            "tutorial logical12 font line advance differs"
        );
        let mut images = std::array::from_fn(|_| None);
        //Captured English cTSFont12 color0xc679. Coverage palette alpha is
        //i/15; native565 quantizes blending, whereas Iced blends RGBA.
        let rgb = rgb565(0xc679);
        for g in &decoded.glyphs {
            if g.code == 32 || g.width == 0 || g.height == 0 {
                continue;
            }
            let mut rgba = Vec::with_capacity(usize::from(g.width) * usize::from(g.height) * 4);
            for y in 0..usize::from(g.height) {
                for x in 0..usize::from(g.width) {
                    let index = (usize::from(g.atlas_y) + y) * usize::from(decoded.width)
                        + usize::from(g.atlas_x)
                        + x;
                    rgba.extend([rgb[0], rgb[1], rgb[2], decoded.coverage[index] * 17]);
                }
            }
            images[usize::from(g.code)] = Some(iced::widget::image::Handle::from_rgba(
                u32::from(g.width),
                u32::from(g.height),
                rgba,
            ));
        }
        let mut encoding = BTreeMap::new();
        for byte in 0..=255u8 {
            let text = sims_formats::strings::decode(&[byte], 1);
            let ch = text
                .chars()
                .next()
                .context("empty font code-page mapping")?;
            encoding.entry(ch).or_insert(byte);
        }
        Ok(Self {
            metrics,
            line_height,
            width_addition,
            decoded,
            encoding,
            images,
        })
    }
    pub fn metric(&self, ch: char) -> Option<Metric> {
        self.encoding
            .get(&ch)
            .map(|b| self.metrics[usize::from(*b)])
    }
    pub fn supports(&self, text: &str) -> bool {
        text.chars().all(|c| self.encoding.contains_key(&c))
    }
    pub fn draw(&self, text: &str, renderer: &mut iced::Renderer, origin: Point, clip: Rectangle) {
        let mut x = origin.x;
        for ch in text.chars() {
            let Some(&byte) = self.encoding.get(&ch) else {
                return;
            };
            let m = self.metrics[usize::from(byte)];
            if let Some(handle) = &self.images[usize::from(byte)] {
                renderer.draw_image(
                    iced::advanced::image::Image {
                        handle: handle.clone(),
                        filter_method: iced::widget::image::FilterMethod::Nearest,
                        rotation: iced::Radians(0.),
                        opacity: 1.,
                        border_radius: Default::default(),
                        snap: true,
                    },
                    Rectangle::new(
                        Point::new(x + m.bearing[0] as f32, origin.y + m.bearing[1] as f32),
                        Size::new(m.ink() as f32, (m.packed[3] - m.packed[1]) as f32),
                    ),
                    clip,
                );
            }
            x += m.advance as f32;
        }
    }
}
fn rgb565(c: u16) -> [u8; 3] {
    let (r, g, b) = (
        ((c >> 11) & 31) as u8,
        ((c >> 5) & 63) as u8,
        (c & 31) as u8,
    );
    [
        (r << 3) | (r >> 2),
        (g << 2) | (g >> 4),
        (b << 3) | (b >> 2),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "private shipped font and preserved read-only original atlas capture"]
    fn bitmap_font_metrics_and_coverage_match_original_atlas() -> Result<()> {
        let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .canonicalize()?;
        let index = sims_content::Index::scan(&repo.join("game-data/cabinet-extracted"))?;
        let font = Font::load(&index)?;
        let capture = repo.join(
            "cache/tutorial-walkthrough-20261022-first/first-run/native-font-atlas-pixels-ui.json",
        );
        let data: serde_json::Value = serde_json::from_slice(&std::fs::read(&capture)?)?;
        ensure!(
            data["tick_before"] == data["tick_after"],
            "native atlas capture crossed a tick"
        );
        let native = data["windows"]
            .as_array()
            .context("native windows absent")?
            .iter()
            .find(|w| w["font_atlas_pixels8_hex"].as_str().is_some())
            .context("native atlas absent")?;
        let unhex = |v: &serde_json::Value| -> Result<Vec<u8>> {
            let s = v.as_str().context("native bytes absent")?;
            ensure!(s.len() % 2 == 0);
            (0..s.len())
                .step_by(2)
                .map(|i| Ok(u8::from_str_radix(&s[i..i + 2], 16)?))
                .collect()
        };
        let glyphs = unhex(&native["font_glyphs_storage"])?;
        let header = unhex(&native["flow_font_candidate_storage"])?;
        let i32_at = |b: &[u8], o: usize| i32::from_le_bytes(b[o..o + 4].try_into().unwrap());
        ensure!(
            i32_at(&header, 0) == 0x677180 && i32_at(&header, 8) == 12 && i32_at(&header, 12) == 0
        );
        ensure!(
            font.line_height == i32_at(&header, 0x18)
                && font.width_addition == i32_at(&header, 0x34)
        );
        for (code, m) in font.metrics.iter().enumerate() {
            let expected = (0..7)
                .map(|i| i32_at(&glyphs, code * 28 + i * 4))
                .collect::<Vec<_>>();
            ensure!(
                [
                    m.packed[0],
                    m.packed[1],
                    m.packed[2],
                    m.packed[3],
                    m.bearing[0],
                    m.bearing[1],
                    m.advance
                ]
                .as_slice()
                    == expected,
                "native glyph metrics differ for{code}"
            );
        }
        let pixels = unhex(&native["font_atlas_pixels8_hex"])?;
        let pitch = native["font_atlas_pitch"]
            .as_u64()
            .context("native pitch absent")? as usize;
        let mut compared = 0;
        for g in &font.decoded.glyphs {
            let m = font.metrics[usize::from(g.code)];
            for y in 0..usize::from(g.height) {
                for x in 0..usize::from(g.width) {
                    let source = font.decoded.coverage[(usize::from(g.atlas_y) + y)
                        * usize::from(font.decoded.width)
                        + usize::from(g.atlas_x)
                        + x];
                    let actual =
                        pixels[(m.packed[1] as usize + y) * pitch + m.packed[0] as usize + x];
                    ensure!(
                        source == actual,
                        "native glyph coverage differs for{}at({x},{y})",
                        g.code
                    );
                    compared += 1;
                }
            }
        }
        ensure!(compared == 26648);
        if let Ok(output) = std::env::var("SIMS_TUTORIAL_FONT_OUTPUT") {
            let path = repo.join(output);
            ensure!(path.starts_with(repo.join("cache")) && !path.exists());
            std::fs::write(
                path,
                serde_json::to_vec_pretty(
                    &serde_json::json!({"native_capture":capture,"tick":data["tick_before"],"glyph_metrics_matched":256,"glyph_coverage_pixels_matched":compared,"line_height":font.line_height,"width_addition":font.width_addition,"body_color565":"0xc679","coverage_alpha":"index*17; RGBA host blending differs from native565 quantization","production_scope":"tutorial body only; primary button label retains prior host font"}),
                )?,
            )?;
        }
        Ok(())
    }
}
