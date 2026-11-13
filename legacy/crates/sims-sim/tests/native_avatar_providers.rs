use serde::Deserialize;
use sims_sim::pose::clothing::catalog_providers::{Entry, collect};

#[derive(Deserialize)]
struct Input {
    resources: Vec<Vec<u8>>,
    entries: Vec<(u32, u32)>,
    seed: Vec<Vec<u8>>,
}
#[derive(Debug, PartialEq, Deserialize)]
struct Output {
    paths: Vec<Vec<u8>>,
    calls: Vec<u32>,
}
#[derive(Deserialize)]
struct Case {
    input: Input,
    original: Output,
}
#[derive(Deserialize)]
struct Report {
    cases: Vec<Case>,
}
#[cfg(feature = "private-fixtures")]
#[test]
fn original_providers_preserve_instance_grouping_flags_paths_and_order() {
    let report: Report =
        serde_json::from_str(include_str!("fixtures/avatar-providers.json")).unwrap();
    assert_eq!(report.cases.len(), 106);
    for (i, case) in report.cases.into_iter().enumerate() {
        let input = case.input;
        let entries: Vec<_> = input
            .entries
            .into_iter()
            .map(|(resource, flags)| Entry { resource, flags })
            .collect();
        let mut output = Output {
            paths: input.seed,
            calls: vec![],
        };
        collect(&entries, &mut output.paths, |id| {
            output.calls.push(id);
            Ok(input.resources[id as usize].clone())
        })
        .unwrap();
        assert_eq!(output, case.original, "case {i}");
    }
}
