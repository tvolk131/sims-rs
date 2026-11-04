use serde::Deserialize;
use serde_json::{Value, json};
use sims_sim::pose::clothing::texture_names::{BMP, Cache, Host};

#[derive(Deserialize)]
struct Input {
    bmp: Vec<Vec<u8>>,
    tga: Vec<Vec<u8>>,
    bmp_error: i32,
    tga_error: i32,
    seed: Vec<Vec<u8>>,
    repeat: usize,
}
struct Services<'a> {
    input: &'a Input,
    events: Vec<Value>,
}
impl Host for Services<'_> {
    fn enumerate(&mut self, kind: u32, names: &mut Vec<Vec<u8>>) -> Result<i32, String> {
        self.events.push(json!(["enumerate", kind]));
        let (values, error) = if kind == BMP {
            (&self.input.bmp, self.input.bmp_error)
        } else {
            (&self.input.tga, self.input.tga_error)
        };
        names.extend(values.iter().cloned());
        Ok(error)
    }
}
#[cfg(feature = "private-fixtures")]
#[test]
fn original_texture_name_cache_retains_order_empty_retry_failure_and_truncation() {
    let report: Value = serde_json::from_str(include_str!("fixtures/texture-names.json")).unwrap();
    assert_eq!(report["cases"].as_array().unwrap().len(), 80);
    for (n, row) in report["cases"].as_array().unwrap().iter().enumerate() {
        let input: Input = serde_json::from_value(row["input"].clone()).unwrap();
        let mut cache = Cache {
            names: input.seed.clone(),
        };
        let mut services = Services {
            input: &input,
            events: vec![],
        };
        let mut names = vec![];
        for _ in 0..input.repeat {
            let empty = cache.names.is_empty();
            cache.load(&mut services).unwrap();
            // Native temporary resource-list disposal is an ownership boundary.
            if empty {
                services.events.push(json!(["dispose_list"]));
            }
            names.push(cache.names.clone());
        }
        assert_eq!(
            json!({"names":names,"events":services.events}),
            row["original"],
            "case {n}"
        );
    }
}
