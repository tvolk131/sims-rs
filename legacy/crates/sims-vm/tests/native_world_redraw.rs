use serde::Deserialize;
use sims_vm::{ObjectId, presenter::redraw};
use std::cell::RefCell;
#[derive(Deserialize)]
struct Object {
    tile: [i8; 3],
    flags: u32,
}
#[derive(Deserialize)]
struct Input {
    size: i32,
    enabled: u8,
    include_visible: u8,
    objects: Vec<Object>,
    order: Vec<ObjectId>,
    floors: Vec<Vec<u8>>,
}
#[derive(Deserialize)]
struct Expected {
    visited: Vec<ObjectId>,
    flags_queries: Vec<ObjectId>,
    compose: Vec<u8>,
}
#[derive(Deserialize)]
struct Row {
    input: Input,
    expected: Expected,
}
#[derive(Deserialize)]
struct Corpus {
    cases: Vec<Row>,
}
struct View<'a> {
    input: &'a Input,
    visited: RefCell<Vec<ObjectId>>,
    flags: RefCell<Vec<ObjectId>>,
}
impl redraw::View for View<'_> {
    fn registered(&self) -> Result<&[ObjectId], String> {
        Ok(&self.input.order)
    }
    fn cached_tile(&self, id: ObjectId) -> Result<[i8; 3], String> {
        self.visited.borrow_mut().push(id);
        Ok(self.input.objects[id as usize - 1].tile)
    }
    fn active_size(&self) -> Result<i32, String> {
        Ok(self.input.size)
    }
    fn native_flags(&self, id: ObjectId) -> Result<u32, String> {
        self.flags.borrow_mut().push(id);
        Ok(self.input.objects[id as usize - 1].flags)
    }
    fn tile_flags(&self, [x, y, z]: [i8; 3]) -> Result<u8, String> {
        Ok(self.input.floors[z as usize - 1][x as usize * self.input.size as usize + y as usize])
    }
}
#[cfg(feature = "private-fixtures")]
#[test]
fn original_redraw_scan_preserves_registry_signed_bounds_flag_and_first_match_order() {
    let corpus: Corpus =
        serde_json::from_str(include_str!("fixtures/native-world-redraw.json")).unwrap();
    assert_eq!(corpus.cases.len(), 512);
    for row in corpus.cases {
        let view = View {
            input: &row.input,
            visited: RefCell::new(vec![]),
            flags: RefCell::new(vec![]),
        };
        let matched = redraw::first(&view, row.input.enabled).unwrap();
        assert_eq!(*view.visited.borrow(), row.expected.visited);
        assert_eq!(*view.flags.borrow(), row.expected.flags_queries);
        let expected =
            (!row.expected.compose.is_empty()).then(|| *row.expected.visited.last().unwrap());
        assert_eq!(matched, expected);
        assert_eq!(
            row.expected.compose,
            if matched.is_some() {
                vec![row.input.include_visible]
            } else {
                vec![]
            }
        );
    }
}
