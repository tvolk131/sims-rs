use serde::Deserialize;
use sims_sim::pose::portrait::border::{self, Scan};

#[derive(Deserialize)]
struct Input {
    pixels: Vec<u16>,
    stride: usize,
    rect: [usize; 4],
    background: u16,
    lock: bool,
}
#[derive(Deserialize)]
struct Case {
    input: Input,
    original: Scan,
}
#[derive(Deserialize)]
struct Report {
    cases: Vec<Case>,
}

#[cfg(feature = "private-fixtures")]
#[test]
fn original_portrait_border_matches_pixels_strides_empty_and_failed_locks() {
    let report: Report =
        serde_json::from_str(include_str!("fixtures/portrait-border.json")).unwrap();
    assert_eq!(report.cases.len(), 144);
    for (n, case) in report.cases.into_iter().enumerate() {
        let c = case.input;
        let before = c.pixels.clone();
        let result = border::scan(
            c.lock.then_some(c.pixels.as_slice()),
            c.stride,
            c.rect,
            &c.background,
        )
        .unwrap();
        assert_eq!(result, case.original, "case {n}");
        assert_eq!(c.pixels, before);
    }
}

#[test]
fn portrait_border_rejects_invalid_surface_storage() {
    for (stride, rect) in [
        (2, [0, 0, 3, 1]),
        (3, [0, 0, 3, 2]),
        (1, [1, 0, 0, 1]),
        (usize::MAX, [0, 0, 1, 3]),
    ] {
        assert!(border::scan(Some(&[0u16; 3]), stride, rect, &0).is_err());
    }
}
