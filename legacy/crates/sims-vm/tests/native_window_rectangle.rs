use serde::Deserialize;
#[derive(Deserialize)]
struct Row {
    rectangle: Option<[i32; 4]>,
    result: u32,
}
#[derive(Deserialize)]
struct Corpus {
    cases: Vec<Row>,
}
#[cfg(feature = "private-fixtures")]
#[test]
fn frozen_wine_rectangle_predicate_preserves_null_signed_and_extreme_edges() {
    let corpus: Corpus =
        serde_json::from_str(include_str!("fixtures/native-window-rectangle.json")).unwrap();
    assert_eq!(corpus.cases.len(), 512);
    for row in corpus.cases {
        assert_eq!(
            sims_vm::ui_effect::widgets::platform::is_rect_empty(row.rectangle),
            row.result
        );
    }
}
