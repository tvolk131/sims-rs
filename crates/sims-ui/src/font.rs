//! The interface's bitmap fonts (`UIGraphics/Fonts/variablesans_*.ffn`),
//! laid out and drawn as the original does.
//!
//! The metrics follow the original's font setup (0x60b97f), as the old
//! prototype ported it for the 12-point font and checked against the
//! glyph table and atlas of the running game
//! (legacy/crates/sims-app/src/tutorial_font.rs):
//!
//! - a glyph's advance is its width plus its x bearing;
//! - the line height is the largest glyph height plus y bearing, and when
//!   a glyph rises above the line (a negative y bearing) every bearing is
//!   moved down by that much, except byte 255's;
//! - the space has no ink, and advances by half the mean ink width.
//!
//! A pixel's coverage is its 4-bit atlas value times 17 (`0x60c446`),
//! blended through [`Frame::blend`] using the original's 16-bit 565 alpha
//! blend table (`0x4999ea` -> `0x6e5eac`, built at `0x49a315`).

use crate::frame::Frame;
use sims_formats::ffn;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Metric {
    /// The glyph in the decoded atlas: x, y, width, height.
    atlas: [u16; 4],
    bearing: [i32; 2],
    advance: i32,
}

/// A font ready to draw.
#[derive(Debug, Clone)]
pub struct Font {
    metrics: Box<[Metric; 256]>,
    /// Characters by the byte the original's code page gives them.
    encoding: std::collections::BTreeMap<char, u8>,
    pub line_height: i32,
    decoded: ffn::Font,
}

/// One pixel of text. The original blends antialiased font coverage
/// (`0..=15`, mapped to `coverage * 17` in the font palette at `0x60c446`)
/// through the same 16-bit 565 alpha blend table (`0x4999ea` -> `0x6e5eac`)
/// used by [`Frame::blend`].
fn ink(frame: &mut Frame, x: i32, y: i32, rgb: [u8; 3], coverage: u8) {
    let c = coverage.min(15);
    if c > 0 {
        frame.blend(x, y, rgb, c * 17);
    }
}

impl Font {
    /// Sets up a parsed font. Glyph codes above 255 are ignored: the
    /// interface's fonts are single-byte.
    pub fn new(decoded: ffn::Font) -> Self {
        let mut metrics = Box::new([Metric::default(); 256]);
        // Where the original packs a glyph in its own atlas does not
        // change how it is drawn, so the decoded atlas is used directly.
        for g in &decoded.glyphs {
            if let Some(m) = metrics.get_mut(usize::from(g.code)) {
                *m = Metric {
                    atlas: [
                        g.atlas_x,
                        g.atlas_y,
                        u16::from(g.width),
                        u16::from(g.height),
                    ],
                    bearing: [i32::from(g.bearing_x), i32::from(g.bearing_y)],
                    advance: i32::from(g.width) + i32::from(g.bearing_x),
                };
            }
        }
        let (mut line_height, mut minimum_y, mut count, mut ink) = (0, 0, 0i32, 0i32);
        for (byte, m) in metrics.iter().enumerate() {
            if byte != 160 && m.atlas[2] > 0 && m.atlas[3] > 0 {
                line_height = line_height.max(i32::from(m.atlas[3]) + m.bearing[1]);
                minimum_y = minimum_y.min(m.bearing[1]);
                count += 1;
                ink += i32::from(m.atlas[2]);
            }
        }
        if minimum_y < 0 {
            line_height -= minimum_y;
            for m in &mut metrics[..255] {
                m.bearing[1] -= minimum_y;
            }
        }
        metrics[32].atlas = [0; 4];
        if count > 0 {
            metrics[32].advance = (f64::from(ink) * 0.5 / f64::from(count) + 0.5).trunc() as i32;
        }
        let mut encoding = std::collections::BTreeMap::new();
        for byte in 0..=255u8 {
            if let Some(ch) = sims_formats::strings::decode(&[byte], 1).chars().next() {
                encoding.entry(ch).or_insert(byte);
            }
        }
        Self {
            metrics,
            encoding,
            line_height,
            decoded,
        }
    }

    fn metric(&self, ch: char) -> Option<&Metric> {
        self.encoding
            .get(&ch)
            .map(|&b| &self.metrics[usize::from(b)])
    }

    /// The width of `text` on one line. Characters the font cannot show
    /// take no room.
    pub fn width(&self, text: &str) -> i32 {
        text.chars()
            .filter_map(|c| self.metric(c))
            .fold(0i32, |w, m| w.saturating_add(m.advance))
    }

    /// Draws `text` on one line with its top-left corner at `at`.
    pub fn draw(&self, frame: &mut Frame, text: &str, at: [i32; 2], rgb: [u8; 3]) {
        self.draw_line(frame, text, at, rgb, false);
    }

    /// Draws `text` as [`Font::draw`] does, underlined as the original
    /// does with its font flag 4 (0x60c136): under each character, from
    /// where it starts to one pixel past its advance, a line in the
    /// text's color on the row just under an `A` (the row 0x60b927
    /// gives: `A`'s y bearing plus its height), drawn only where that
    /// row is inside the line.
    pub fn draw_underlined(&self, frame: &mut Frame, text: &str, at: [i32; 2], rgb: [u8; 3]) {
        self.draw_line(frame, text, at, rgb, true);
    }

    fn draw_line(
        &self,
        frame: &mut Frame,
        text: &str,
        at: [i32; 2],
        rgb: [u8; 3],
        underline: bool,
    ) {
        let a = &self.metrics[usize::from(b'A')];
        let below = a.bearing[1] + i32::from(a.atlas[3]);
        let mut x = at[0];
        for ch in text.chars() {
            let Some(m) = self.metric(ch) else {
                continue;
            };
            let [ax, ay, w, h] = m.atlas.map(usize::from);
            for gy in 0..h {
                for gx in 0..w {
                    let i = (ay + gy) * usize::from(self.decoded.width) + ax + gx;
                    let coverage = self.decoded.coverage.get(i).copied().unwrap_or(0);
                    ink(
                        frame,
                        x.saturating_add(m.bearing[0]).saturating_add(gx as i32),
                        at[1].saturating_add(m.bearing[1]).saturating_add(gy as i32),
                        rgb,
                        coverage,
                    );
                }
            }
            if underline && below < self.line_height {
                frame.fill(
                    crate::frame::Rect::new(
                        x,
                        at[1].saturating_add(below),
                        m.advance.saturating_add(1),
                        1,
                    ),
                    rgb,
                );
            }
            x = x.saturating_add(m.advance);
        }
    }

    /// Breaks `text` into the lines the original's wrapped text gives in
    /// a box `width` wide (0x60b4a1, fitting each line with 0x60b620).
    /// A line is measured by the advances of its characters but the
    /// last, plus that one's glyph width. It ends after the last space or
    /// hyphen that fits, or mid-word when there is none (one character
    /// before the first that does not fit), or after a line break; a
    /// first character too wide for the box takes its whole word. The
    /// spaces after a line's end stay on it. Lines here drop their line
    /// break. Characters the font cannot show take no room.
    ///
    /// The original also stops at a line that starts with a character
    /// outside the font's range (0x60b898); the range is **unverified**,
    /// so here only the end of the text stops it.
    pub fn break_lines(&self, text: &str, width: i32) -> Vec<String> {
        let chars: Vec<char> = text.chars().collect();
        let mut lines = Vec::new();
        let mut start = 0;
        while start < chars.len() {
            let rest = &chars[start..];
            let n = self.fit(rest, width).clamp(1, rest.len());
            lines.push(rest[..n].iter().filter(|&&c| c != '\n').collect());
            start += n;
        }
        lines
    }

    /// How many characters of `text` start a line `width` wide (0x60b620),
    /// by the rules [`Font::break_lines`] gives; 0 for no text.
    pub fn fit(&self, text: &[char], width: i32) -> usize {
        let glyph = |c: char| self.metric(c).map_or(0, |m| i32::from(m.atlas[2]));
        let advance = |c: char| self.metric(c).map_or(0, |m| m.advance);
        let Some(&first) = text.first() else {
            return 0;
        };
        let (mut end, mut broken) = (0, false);
        let mut measured = glyph(first);
        let mut whole_word = measured > width;
        if !whole_word {
            let mut i = 0;
            loop {
                i += 1;
                if i >= text.len() {
                    return i;
                }
                let c = text[i - 1];
                if c == '\n' {
                    (end, broken) = (i, true);
                    break;
                }
                if matches!(c, ' ' | '-') && measured <= width {
                    end = i;
                }
                // The original's 32-bit sum wraps.
                measured = measured
                    .wrapping_sub(glyph(c))
                    .wrapping_add(advance(c))
                    .wrapping_add(glyph(text[i]));
                if measured > width {
                    break;
                }
            }
            if end == 0 {
                if i > 1 {
                    end = i - 1;
                } else {
                    whole_word = true;
                }
            }
        }
        if whole_word {
            while end < text.len() && !matches!(text[end], '\0' | '\n' | ' ') {
                end += 1;
            }
        }
        if !broken {
            while end < text.len() && text[end] == ' ' {
                end += 1;
            }
        }
        end.max(1)
    }

    /// Breaks `text` into lines no wider than `width`, at spaces where it
    /// can; a word wider than a line stands alone. Line breaks in the
    /// text are kept.
    pub fn wrap(&self, text: &str, width: i32) -> Vec<String> {
        let mut lines = Vec::new();
        for paragraph in text.split('\n') {
            let mut line = String::new();
            for word in paragraph.split(' ') {
                let candidate = if line.is_empty() {
                    word.to_owned()
                } else {
                    format!("{line} {word}")
                };
                if self.width(&candidate) <= width || line.is_empty() {
                    line = candidate;
                } else {
                    lines.push(std::mem::replace(&mut line, word.to_owned()));
                }
            }
            lines.push(line);
        }
        lines
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Two glyphs, `A` (2 by 2, rising one pixel above the line) and `B`
    /// (1 by 1), and a space.
    pub(crate) fn font() -> Font {
        Font::new(ffn::Font {
            flags: 9,
            glyphs: vec![
                ffn::Glyph {
                    code: 65,
                    width: 2,
                    height: 2,
                    atlas_x: 0,
                    atlas_y: 0,
                    first_bearing: 0,
                    bearing_x: 1,
                    bearing_y: -1,
                },
                ffn::Glyph {
                    code: 66,
                    width: 1,
                    height: 1,
                    atlas_x: 2,
                    atlas_y: 0,
                    first_bearing: 0,
                    bearing_x: 0,
                    bearing_y: 0,
                },
                ffn::Glyph {
                    code: 300,
                    width: 1,
                    height: 1,
                    atlas_x: 0,
                    atlas_y: 0,
                    first_bearing: 0,
                    bearing_x: 0,
                    bearing_y: 0,
                },
            ],
            width: 3,
            height: 2,
            coverage: vec![15, 0, 15, 15, 15, 0],
        })
    }

    /// [`font`] with a `g`, 1 by 2, hanging two pixels below `A`'s top,
    /// so the line is 4 high and a row is left under `A`.
    pub(crate) fn hanging_font() -> Font {
        let mut decoded = font().decoded;
        decoded.glyphs.push(ffn::Glyph {
            code: 103,
            width: 1,
            height: 2,
            atlas_x: 0,
            atlas_y: 0,
            first_bearing: 0,
            bearing_x: 0,
            bearing_y: 1,
        });
        Font::new(decoded)
    }

    #[test]
    fn metrics_follow_the_original_rules() {
        let f = font();
        // Tallest glyph plus bearing is 1; moving everything down by the
        // one pixel A rises gives 2.
        assert_eq!(f.line_height, 2);
        assert_eq!(f.width("AB"), 3 + 1);
        // The space: half the mean ink width (1.5 / 2), rounded.
        assert_eq!(f.width(" "), 1);
        assert_eq!(f.width("A\u{1F600}"), 3);
    }

    #[test]
    fn an_underline_runs_under_each_character_on_the_row_under_an_a() {
        // The test font's `A` reaches the line's bottom: no row is left
        // under it, so nothing is underlined.
        let f = font();
        let mut plain = Frame::new(6, 3, [0, 0, 0]);
        f.draw(&mut plain, "AB", [0, 0], [255, 255, 255]);
        let mut under = Frame::new(6, 3, [0, 0, 0]);
        f.draw_underlined(&mut under, "AB", [0, 0], [255, 255, 255]);
        assert_eq!(under.rgb(), plain.rgb());
        // A `g` hanging two pixels lower leaves room: the line is the row
        // under `A`, from each character's start to a pixel past its
        // advance (A's 3, B's 1).
        let f = hanging_font();
        assert_eq!(f.line_height, 4);
        let mut frame = Frame::new(8, 5, [0, 0, 0]);
        f.draw_underlined(&mut frame, "AB", [1, 1], [9, 9, 9]);
        let row: Vec<_> = (0..8).map(|x| frame.pixel(x, 3).unwrap()).collect();
        assert_eq!(
            row,
            [
                [0; 3], [9; 3], [9; 3], [9; 3], [9; 3], [9; 3], [0; 3], [0; 3]
            ]
        );
        // Far coordinates do not overflow.
        f.draw_underlined(&mut frame, "AB", [i32::MAX - 1, i32::MAX - 2], [1; 3]);
    }

    #[test]
    fn draw_and_wrap() {
        let f = font();
        let mut frame = Frame::new(6, 3, [0, 0, 0]);
        f.draw(&mut frame, "AB", [0, 0], [255, 255, 255]);
        // A's first pixel lands one right (bearing) and at the top.
        assert_eq!(frame.pixel(1, 0), Some([255, 255, 255]));
        assert_eq!(frame.pixel(2, 0), Some([0, 0, 0]));
        assert_eq!(frame.pixel(3, 1), Some([255, 255, 255]));
        assert_eq!(f.wrap("AB AB AB", 9), ["AB AB", "AB"]);
        assert_eq!(f.wrap("ABABAB B", 2), ["ABABAB", "B"]);
        f.draw(&mut frame, "AAAA", [i32::MAX - 2, i32::MAX - 1], [1, 1, 1]);
    }

    #[test]
    fn lines_break_as_the_original_fits_them() {
        let f = font();
        // A takes 3 (2 of glyph), B 1, the space 1 and no glyph; the
        // hyphen is not in the font.
        let lines = |text: &str, width: i32| f.break_lines(text, width);
        // After a hyphen that fits; the A after it would reach 6.
        assert_eq!(lines("AB-AB", 5), ["AB-", "AB"]);
        // After a space, which stays on its line.
        assert_eq!(lines("AB AB", 5), ["AB ", "AB"]);
        assert_eq!(lines("AB  AB", 5), ["AB  ", "AB"]);
        // Mid-word, one character before the one that does not fit; the
        // last two fit (3 + 2).
        assert_eq!(lines("AAAA", 5), ["A", "A", "AA"]);
        // A first glyph wider than the box takes its whole word.
        assert_eq!(lines("AA B", 1), ["AA ", "B"]);
        assert_eq!(lines("A\nB", 9), ["A", "B"]);
        assert!(lines("", 5).is_empty());
    }

    #[test]
    fn a_line_measure_wraps_as_the_originals_does() {
        // 255 wide and 382 to the next: about 5.6 million of them pass
        // i32::MAX, where the original's sum wraps negative and keeps
        // fitting.
        let f = Font::new(ffn::Font {
            flags: 9,
            glyphs: vec![ffn::Glyph {
                code: 65,
                width: 255,
                height: 1,
                atlas_x: 0,
                atlas_y: 0,
                first_bearing: 0,
                bearing_x: 127,
                bearing_y: 0,
            }],
            width: 255,
            height: 1,
            coverage: vec![15; 255],
        });
        let n = usize::try_from(i32::MAX / 382).unwrap() + 10;
        let lines = f.break_lines(&"A".repeat(n), i32::MAX);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].len(), n);
    }
}
