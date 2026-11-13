use serde_json::Value;
use sims_sim::object_catalog::move_tool::Fields;

#[cfg(feature = "private-fixtures")]
#[test]
fn move_camera_callback_matches_original_and_preserves_other_fields() {
    let fixture: Value =
        serde_json::from_str(include_str!("fixtures/camera-selected-tool.json")).unwrap();
    assert_eq!(
        fixture["source_sha256"],
        "32857c25e00594a9bc5d700f7b373b6df59ab76ec51b36aa1d30f3dd973eff58"
    );
    assert_eq!(
        fixture["capture_sha256"],
        "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
    );
    let rows = fixture["cases"].as_array().unwrap();
    assert_eq!(rows.len(), 184);
    let mut compared = 0;
    for row in rows.iter().filter(|r| r["input"]["receiver"] == "move") {
        let c = &row["input"];
        let mut f = Fields::constructor_fields();
        f.mode = c["mode"].as_i64().unwrap() as i32;
        f.position = serde_json::from_value(c["position"].clone()).unwrap();
        f.cursor = 123;
        f.special = 23;
        f.flags = [5, 4, 3, 2, 1];
        f.definition = Some(1234);
        let mut expected = f.clone();
        expected.position = serde_json::from_value(row["original"]["position"].clone()).unwrap();
        f.camera_changed(c["held"].as_bool().unwrap()).unwrap();
        assert_eq!(f, expected, "{c}");
        compared += 1;
    }
    assert_eq!(compared, 56);
}

#[test]
fn special_held_offsets_stay_explicit_and_unchanged_on_failure() {
    for mode in [3, 4] {
        let mut f = Fields::constructor_fields();
        f.mode = mode;
        f.position = [19, -37];
        let before = f.clone();
        assert!(f.camera_changed(true).unwrap_err().contains("door/window"));
        assert_eq!(f, before);
        f.camera_changed(false).unwrap();
        assert_eq!(f, before);
    }
}

#[cfg(feature = "private-fixtures")]
#[test]
fn zoom_buttons_match_original_clamping_deferred_order_and_selected_tool() {
    use serde_json::json;
    use sims_sim::lot_presenter::{
        camera_command::{Request, Ui, Zoom},
        open,
    };
    let f: Value = serde_json::from_str(include_str!("fixtures/camera-zoom-buttons.json")).unwrap();
    assert_eq!(
        f["source_sha256"],
        "32857c25e00594a9bc5d700f7b373b6df59ab76ec51b36aa1d30f3dd973eff58"
    );
    assert_eq!(
        f["capture_sha256"],
        "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
    );
    let cases = f["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 96);
    for row in cases {
        let c = &row["input"];
        let mut ui = Ui {
            flags: 0xa5,
            zoom: c["zoom"].as_u64().unwrap() as u32,
            ..Default::default()
        };
        let mut tool = Fields::constructor_fields();
        tool.position = [-20, -1];
        let mut call = Zoom::step(
            &mut ui,
            c["inward"].as_bool().unwrap(),
            c["house"].as_i64().unwrap() as i16,
            c["deferred"].as_u64().unwrap() as u32,
        );
        let mut events = vec![];
        while let Some(request) = call.request().cloned() {
            call = serde_json::from_value(serde_json::to_value(call).unwrap()).unwrap();
            call.validate().unwrap();
            let state = json!({"flags":ui.flags,"zoom":ui.zoom});
            events.push(match request {
                Request::Camera(open::Request::Zoom { zoom, force, refresh_ui }) => json!({"call":"camera","args":[zoom,u32::from(force),u32::from(refresh_ui)],"state":state}),
                Request::RefreshAudio => json!({"call":"audio","state":state}),
                Request::RefreshControls => {
                    tool.camera_changed(true).unwrap();
                    json!({"call":"selected-tool","state":state})
                }
                _ => panic!("unrelated zoom button request"),
            });
            call.acknowledge_with_ui(&request, &mut ui).unwrap();
        }
        call.validate().unwrap();
        assert_eq!(
            json!({"events":events,"state":{"flags":ui.flags,"zoom":ui.zoom},"position":tool.position}),
            row["original"],
            "{c}"
        );
    }
}
