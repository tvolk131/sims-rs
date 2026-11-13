//! Read-only5134bf bar extent and5139dc static palette. The native timer-based
//! flashes are not hosted here; drawing never drives a timer or VM update.
use super::*;
use iced::advanced::{Widget, layout, renderer, widget::Tree};

pub(crate) const FONT: iced::Font = iced::Font {
    family: iced::font::Family::Name("Comic Sans MS"),
    ..iced::Font::DEFAULT
};
#[derive(Debug, PartialEq, Eq)]
struct Paint {
    width: u32,
    left: [u8; 3],
    right: [u8; 3],
}
fn mix(end: [u8; 3], t: f32) -> [u8; 3] {
    //5137e2 first quantizes the factor to an unsigned byte, then uses the
    //stored reciprocal255 in x87 and rounds each resulting color channel.
    let step = (f64::from(t) * 255. + 0.5).trunc() as u8;
    let factor = f64::from(step) * f64::from(1.0f32 / 255.);
    let start = [64., 93., 95.];
    std::array::from_fn(|i| {
        (start[i] + (f64::from(end[i]) - start[i]) * factor + 0.5).trunc() as u8
    })
}
fn paint(value: i32) -> Paint {
    let factor = f64::from(value + 50) * f64::from(0.01f32);
    Paint {
        width: ((f64::from(value + 100) * f64::from(0.3f32) + 0.5).trunc() as u32).min(60),
        left: mix([0, 202, 57], (factor as f32).clamp(0., 1.)),
        right: mix([209, 0, 0], (1. - factor.clamp(0., 1.)) as f32),
    }
}
pub(crate) struct Meter(pub f32);
impl Widget<Message, iced::Theme, iced::Renderer> for Meter {
    fn size(&self) -> Size<iced::Length> {
        Size::new(60.into(), 5.into())
    }
    fn layout(
        &mut self,
        _: &mut Tree,
        _: &iced::Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        layout::Node::new(limits.resolve(60, 5, Size::new(60., 5.)))
    }
    fn draw(
        &self,
        _: &Tree,
        r: &mut iced::Renderer,
        _: &iced::Theme,
        _: &renderer::Style,
        l: iced::advanced::Layout<'_>,
        _: iced::mouse::Cursor,
        _: &iced::Rectangle,
    ) {
        use iced::advanced::Renderer as _;
        let p = paint(self.0.clamp(-100., 100.).trunc() as i32);
        let b = l.bounds();
        let color = |rgb: [u8; 3]| {
            Color::from_rgb8(
                ((u16::from(rgb[0]) >> 3) * 255 / 31) as u8,
                ((u16::from(rgb[1]) >> 2) * 255 / 63) as u8,
                ((u16::from(rgb[2]) >> 3) * 255 / 31) as u8,
            )
        };
        r.fill_quad(
            renderer::Quad {
                bounds: b,
                ..Default::default()
            },
            color(p.right),
        );
        r.fill_quad(
            renderer::Quad {
                bounds: iced::Rectangle {
                    width: p.width as f32,
                    ..b
                },
                ..Default::default()
            },
            color(p.left),
        );
        //51376e draws the one-pixel divider only with at least two pixels left.
        if p.width > 0 && 60 - p.width > 1 {
            r.fill_quad(
                renderer::Quad {
                    bounds: iced::Rectangle {
                        x: b.x + p.width as f32 - 1.,
                        width: 1.,
                        ..b
                    },
                    ..Default::default()
                },
                Color::BLACK,
            );
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(feature = "private-fixtures")]
    #[test]
    fn every_integer_motive_matches_original_static_colors_and_extent() {
        let report: serde_json::Value =
            serde_json::from_str(include_str!("../tests/fixtures/motive-paint.json")).unwrap();
        for c in report["cases"].as_array().unwrap() {
            let n = c["motive"].as_i64().unwrap() as i32;
            assert_eq!(
                paint(n),
                Paint {
                    width: c["width"].as_u64().unwrap() as u32,
                    left: serde_json::from_value(c["left"].clone()).unwrap(),
                    right: serde_json::from_value(c["right"].clone()).unwrap()
                },
                "motive{n}"
            );
        }
    }
}
