use serde::Deserialize;
use sims_vm::dialog::{Effect, Input, Protocol};

#[cfg(feature = "private-fixtures")]
#[test]
fn career_tokens_match_880_native_cases() {
    use sims_vm::{
        career_text::{Level, Text},
        dialog::{format_careers, format_locals},
        program::CareerCatalog,
    };
    let report: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/dialog-career.json")).unwrap();
    assert_eq!(
        report["capture_sha256"],
        "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
    );
    assert_eq!(
        report["source_sha256"],
        "32857c25e00594a9bc5d700f7b373b6df59ab76ec51b36aa1d30f3dd973eff58"
    );
    let records = report["records"].as_array().unwrap();
    let catalog = CareerCatalog {
        ids: records
            .iter()
            .map(|r| r["id"].as_i64().unwrap() as i32)
            .collect(),
        text: Some(
            records
                .iter()
                .map(|r| Text {
                    name: String::new(),
                    offers: serde_json::from_value(r["offers"].clone()).unwrap(),
                    levels: r["levels"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|level| Level {
                            titles: serde_json::from_value(level["titles"].clone()).unwrap(),
                            secondary: Default::default(),
                            description: level["description"].as_str().unwrap().into(),
                        })
                        .collect(),
                })
                .collect(),
        ),
        ..Default::default()
    };
    let cases = report["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 880);
    for row in cases {
        let c = &row["input"];
        let locals: Vec<i16> = serde_json::from_value(c["locals"].clone()).unwrap();
        let locals = c["frame"].as_bool().unwrap().then_some(locals.as_slice());
        let text = format_locals(c["text"].as_str().unwrap(), locals).unwrap();
        let actual = format_careers(
            &text,
            &catalog,
            serde_json::from_value(c["person"].clone()).unwrap(),
            locals,
        )
        .unwrap();
        assert_eq!(actual, row["original"]["text"], "{c}");
    }
}

#[cfg(feature = "private-fixtures")]
#[test]
fn local_tokens_and_job_icons_match_native() {
    use sims_vm::dialog::{format_locals, job_icon_rect};
    let r: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/dialog-local.json")).unwrap();
    assert_eq!(r["cases"].as_array().unwrap().len(), 126);
    for row in r["cases"].as_array().unwrap() {
        let c = &row["input"];
        let locals: Vec<i16> = serde_json::from_value(c["locals"].clone()).unwrap();
        let text = format_locals(
            c["text"].as_str().unwrap(),
            c["frame"].as_bool().unwrap().then_some(locals.as_slice()),
        )
        .unwrap();
        assert_eq!(text, row["original"].as_str().unwrap(), "{c}");
    }
    let r: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/dialog-job-icon.json")).unwrap();
    assert_eq!(r["cases"].as_array().unwrap().len(), 108);
    for row in r["cases"].as_array().unwrap() {
        let c = &row["input"];
        let rect = job_icon_rect(
            c["job"].as_i64().unwrap() as i32,
            serde_json::from_value(c["bounds"].clone()).unwrap(),
        );
        assert_eq!(serde_json::json!(rect), row["original"], "{c}");
    }
}

#[derive(Deserialize)]
struct CaseInput {
    kind: u16,
    stack_object: i16,
    modal: bool,
    special_busy: bool,
    words: [i16; 10],
    owners: [i16; 3],
    status: Option<u32>,
    operands: [u8; 8],
}
#[derive(Deserialize)]
struct Original {
    error: Option<u32>,
    returned: Option<u8>,
    temps: [i16; 8],
    words: [i16; 10],
    owners: [i16; 3],
    status: Option<u32>,
    calls: Vec<String>,
}
#[derive(Deserialize)]
struct Case {
    input: CaseInput,
    original: Original,
}
#[derive(Deserialize)]
struct Report {
    cases: Vec<Case>,
    capture_sha256: String,
    source_sha256: String,
}

#[cfg(feature = "private-fixtures")]
#[test]
fn dialog_protocol_matches_840_original_cases() {
    let report: Report = serde_json::from_str(include_str!("fixtures/dialog.json")).unwrap();
    assert_eq!(report.cases.len(), 840);
    assert_eq!(
        report.capture_sha256,
        "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
    );
    assert_eq!(
        report.source_sha256,
        "32857c25e00594a9bc5d700f7b373b6df59ab76ec51b36aa1d30f3dd973eff58"
    );
    for (n, case) in report.cases.into_iter().enumerate() {
        let c = case.input;
        let expected = case.original;
        let mut protocol = Protocol {
            words: c.words,
            owners: c.owners.map(|id| (id != 0).then_some(id)),
        };
        let mut temps = [11, 22, 33, 44, 2, 66, 77, 88];
        let mut status = c.status;
        let result = protocol.step(
            Input {
                caller: 1,
                caller_kind: c.kind,
                caller_neighbor: 37,
                stack_person: matches!(c.stack_object, 1 | 2).then_some(c.stack_object),
                operands: c.operands,
                modal_busy: c.modal,
                special_busy: c.special_busy,
            },
            &mut temps,
            &mut status,
        );
        if let Some(error) = expected.error {
            assert_eq!(error, 0x1c, "case {n}");
            assert!(result.unwrap_err().contains("0x1c"), "case {n}");
        } else {
            let step = result.unwrap_or_else(|e| panic!("case {n}: {e}"));
            assert_eq!(Some(step.returned), expected.returned, "case {n}");
            assert_eq!(
                step.effects
                    .iter()
                    .map(|e| match e {
                        Effect::Create => "create",
                        Effect::Close => "close",
                        Effect::Acknowledge => "acknowledge",
                        Effect::CopyInput => "input",
                    })
                    .collect::<Vec<_>>(),
                expected.calls,
                "case {n}"
            );
        }
        assert_eq!(temps, expected.temps, "case {n}");
        assert_eq!(protocol.words, expected.words, "case {n}");
        assert_eq!(
            protocol.owners.map(|id| id.unwrap_or(0)),
            expected.owners,
            "case {n}"
        );
        assert_eq!(status, expected.status, "case {n}");
    }
}

#[cfg(feature = "private-fixtures")]
#[test]
fn dialog_services_match_original_text_responses_and_registry() {
    use sims_vm::{dialog::*, *};
    let r: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/dialog-services.json")).unwrap();
    assert_eq!(r["selections"].as_array().unwrap().len(), 216);
    for row in r["selections"].as_array().unwrap() {
        let c = &row["input"];
        let marked = c["mark"] == true && c["comment"].as_str().is_some_and(|s| s.starts_with('!'));
        assert_eq!(
            select_text(c["text"].as_str(), marked, c["fallback"].as_str()),
            row["original"]["text"],
            "{c}"
        );
        let lookups = &row["original"]["lookups"];
        assert_eq!(lookups[0][1], c["index"]);
        assert_eq!(lookups[1][1], c["index"]);
    }
    assert_eq!(r["names"].as_array().unwrap().len(), 20);
    for row in r["names"].as_array().unwrap() {
        let c = &row["input"];
        let (text, icon) = format_names(
            c["text"].as_str().unwrap(),
            c["caller"].as_str(),
            c["object"].as_str(),
        )
        .unwrap();
        assert_eq!(text, row["original"]["text"], "{c}");
        assert_eq!(icon, row["original"]["icon"].as_bool().unwrap(), "{c}");
    }
    assert_eq!(r["responses"].as_array().unwrap().len(), 144);
    for row in r["responses"].as_array().unwrap() {
        let c = &row["input"];
        assert_eq!(
            response_status(
                c["status"].as_u64().unwrap() as u32,
                c["result"].as_u64().unwrap() as u32,
                c["immediate"] == true
            ),
            row["original"]["status"].as_u64().unwrap() as u32,
            "{c}"
        );
    }
    assert_eq!(r["registry"].as_array().unwrap().len(), 24);
    for row in r["registry"].as_array().unwrap() {
        let c = &row["input"];
        let mut vm = Vm::new(World::default(), 1);
        for (i, status) in c["status"].as_array().unwrap().iter().enumerate() {
            vm.world.dialogs.records.push(Record {
                presentation: None,
                portrait: None,
                bitmap_icon: None,
                global_bitmap_icon: None,
                guid_icon: None,
                id: 1 + i as u64,
                caller: 1,
                origin: Location {
                    source: "synthetic".into(),
                    bhav: 4096,
                    instruction: 0,
                },
                source: "synthetic".into(),
                operands: [0; 8],
                stack_object: 0,
                title: String::new(),
                body: String::new(),
                buttons: vec![],
                icon_object: None,
                job_icon: None,
                status: status.as_u64().unwrap() as u32,
            });
            vm.world.ui_owner.dialog_owners.push(1);
        }
        let mut shown = vec![];
        while let Some(id) = vm.pump_dialogs().unwrap() {
            shown.push(serde_json::json!(["show", id - 1]));
            // Supply the same widget-call result as the native probe, including
            // return-without-response status 1. No UI button input is synthesized.
            vm.world
                .dialogs
                .records
                .iter_mut()
                .find(|r| r.id == id)
                .unwrap()
                .status = c["answers"][(id - 1) as usize].as_u64().unwrap() as u32;
            vm.world.dialogs.modal = None;
        }
        assert_eq!(
            serde_json::json!(
                vm.world
                    .dialogs
                    .records
                    .iter()
                    .map(|r| r.id - 1)
                    .collect::<Vec<_>>()
            ),
            row["original"]["ids"],
            "{c}"
        );
        assert_eq!(
            serde_json::json!(
                vm.world
                    .dialogs
                    .records
                    .iter()
                    .map(|r| r.status)
                    .collect::<Vec<_>>()
            ),
            row["original"]["status"],
            "{c}"
        );
        let expected = row["original"]["events"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|e| e[0] == "show")
            .cloned()
            .collect::<Vec<_>>();
        assert_eq!(shown, expected, "{c}");
    }
}
