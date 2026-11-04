use serde::Deserialize;
use sims_render::censor::{self, Mosaic};
#[derive(Deserialize)]
struct Input {
    width: usize,
    height: usize,
    format: i32,
    zoom: i32,
    rect: [i32; 4],
    clip: [i32; 4],
    pixels: Vec<u16>,
    depth: Vec<u16>,
    thresholds: Vec<i32>,
    noise: Vec<u32>,
}
#[derive(Deserialize)]
struct Case {
    input: Input,
    pixels: Vec<u16>,
    noise_calls: usize,
    depth_rows: Vec<usize>,
}
#[derive(Deserialize)]
struct Corpus {
    cases: Vec<Case>,
}
#[cfg(feature = "private-fixtures")]
#[test]
fn mosaic_matches_original_pixels_depth_queries_and_noise_consumption() {
    let corpus: Corpus =
        serde_json::from_str(include_str!("fixtures/native-censor-pixels.json")).unwrap();
    for (case_number, case) in corpus.cases.into_iter().enumerate() {
        let i = case.input;
        assert_eq!(i.pixels.len(), i.width * i.height);
        let mut pixels = i.pixels;
        let mut draws = 0;
        let mut queries = vec![];
        censor::apply(
            &mut pixels,
            &i.depth,
            i.width,
            Mosaic {
                rect: i.rect,
                clip: i.clip,
                zoom: i.zoom,
                format: i.format,
            },
            |row| {
                queries.push(row);
                i.thresholds[row]
            },
            || {
                let value = i.noise[draws % i.noise.len()];
                draws += 1;
                value
            },
        )
        .unwrap();
        assert_eq!(pixels, case.pixels, "pixels in case {case_number}");
        assert_eq!(
            draws, case.noise_calls,
            "noise consumption in case {case_number}"
        );
        assert_eq!(
            queries, case.depth_rows,
            "depth query order in case {case_number}"
        );
    }
}
