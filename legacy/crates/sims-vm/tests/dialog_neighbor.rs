use serde::Deserialize;
use serde_json::{Value, json};
use sims_vm::dialog::format_neighbor;
use std::cell::RefCell;

#[derive(Deserialize)]
struct Fixture {
    cases: Vec<Row>,
}
#[derive(Deserialize)]
struct Row {
    input: Input,
    original: Value,
}
#[derive(Deserialize)]
struct Input {
    text: String,
    name: String,
    stack: i16,
    resource: bool,
    neighbor_output: bool,
    resource_output: bool,
}

#[cfg(feature = "private-fixtures")]
#[test]
fn neighbor_pass_matches_native_text_lookup_and_independent_outputs() {
    let fixture: Fixture =
        serde_json::from_str(include_str!("fixtures/dialog-neighbor.json")).unwrap();
    assert_eq!(fixture.cases.len(), 1896);
    for row in fixture.cases {
        let c = row.input;
        let events = RefCell::new(vec![]);
        let mut neighbor = -17_i16;
        let mut resource = 0_u32;
        let text = format_neighbor(
            &c.text,
            c.stack,
            c.neighbor_output.then_some(&mut neighbor),
            c.resource_output.then_some(&mut resource),
            |id| {
                events.borrow_mut().push(json!(["lookup", id]));
                Ok((c.resource && [1, 3].contains(&id)).then_some(1))
            },
            |_| {
                events.borrow_mut().push(json!(["name"]));
                Ok(c.name.clone())
            },
        )
        .unwrap();
        let words = [neighbor.to_le_bytes(), 29_i16.to_le_bytes()].concat();
        assert_eq!(
            json!({"text":text,"resource_selected":resource==1,"neighbor":neighbor,
            "neighbor_words":words,"events":events.into_inner()}),
            row.original
        );
    }
}

#[test]
fn unresolved_name_service_stops_after_resource_output_and_expansion_is_bounded() {
    let mut neighbor = -17;
    let mut resource = 0;
    let error = format_neighbor(
        "$Neighbor",
        48,
        Some(&mut neighbor),
        Some(&mut resource),
        |_| Ok(Some(9)),
        |_| Err("catalog service unavailable".into()),
    )
    .unwrap_err();
    assert_eq!(error, "catalog service unavailable");
    assert_eq!((neighbor, resource), (-17, 9));
    assert!(
        format_neighbor(
            "$Neighbor",
            48,
            None,
            None,
            |_| Ok(Some(9)),
            |_| Ok("A$Neighbor".into())
        )
        .unwrap_err()
        .contains("exceeds limit")
    );
}
