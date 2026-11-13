use serde::Deserialize;
#[derive(Deserialize)]
struct Case {
    name: String,
    kind: String,
    #[serde(default)]
    words: Vec<u32>,
}
#[derive(Deserialize)]
struct Corpus {
    cases: Vec<Case>,
}

#[cfg(feature = "private-fixtures")]
#[test]
fn original_material_dispatch_and_exact_float_colors() {
    let corpus: Corpus =
        serde_json::from_str(include_str!("fixtures/native-builtin-material.json")).unwrap();
    for case in corpus.cases {
        let material = sims_content::avatar_material::builtin(&case.name);
        if case.kind == "texture" {
            assert!(
                material.is_none(),
                "{} unexpectedly treated as built-in",
                case.name
            );
            continue;
        }
        let material = material.unwrap();
        // Native device ramp size is supplied as 32 by this bounded probe.
        assert_eq!(material.words(32).as_slice(), case.words, "{}", case.name);
    }
}
