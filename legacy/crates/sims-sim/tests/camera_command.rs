#![cfg(feature = "private-fixtures")]

use serde_json::{Value, json};
use sims_sim::lot_presenter::{
    camera_command::{self, Request, Ui, Zoom},
    open,
};

#[test]
fn publication_camera_has_no_ui_setter_writes_or_completion_tail() {
    for request in [
        open::Request::Zoom {
            zoom: 2,
            force: 0,
            refresh_ui: 1,
        },
        open::Request::Rotate {
            rotation: 3,
            force: 0,
            refresh_ui: 1,
        },
        open::Request::Floor { level: 2, force: 0 },
        open::Request::WallsDown(1),
        open::Request::Cutaway(0),
        open::Request::Roof {
            enabled: 0,
            force: 0,
        },
    ] {
        let mut ui = Ui {
            flags: 0x25,
            zoom: 1,
            rotation: 1,
            floor: 1,
        };
        let original = ui;
        let mut c = Zoom::publication(request.clone()).unwrap();
        c = serde_json::from_slice(&serde_json::to_vec(&c).unwrap()).unwrap();
        c.validate().unwrap();
        c.acknowledge_with_ui(&Request::Camera(request), &mut ui)
            .unwrap();
        assert!(c.complete());
        assert_eq!(ui, original);
    }
    assert!(Zoom::publication(open::Request::DrainIndicators).is_err());
}

fn fixture() -> Value {
    let r: Value = serde_json::from_str(include_str!("fixtures/camera-command.json")).unwrap();
    assert_eq!(
        r["source_sha256"],
        "32857c25e00594a9bc5d700f7b373b6df59ab76ec51b36aa1d30f3dd973eff58"
    );
    assert_eq!(
        r["capture_sha256"],
        "d8d7cb4cd754146ffef15c7627c5ed3ed5077972b9b38e8180b1b9372bf2b3fb"
    );
    r
}
#[test]
fn ui_zoom_setter_matches_original_order_and_deferred_calls() {
    let f = fixture();
    let rows = f["commands"].as_array().unwrap();
    assert_eq!(rows.len(), 144);
    for row in rows {
        let c = &row["input"];
        let mut ui = Ui {
            flags: c["flags"].as_u64().unwrap() as u8,
            zoom: 777,
            ..Default::default()
        };
        let mut cursor = Zoom::begin(
            &mut ui,
            c["zoom"].as_u64().unwrap() as u32,
            c["deferred"].as_u64().unwrap() as u32,
        );
        let mut events = vec![];
        while let Some(request) = cursor.request().cloned() {
            cursor = serde_json::from_slice(&serde_json::to_vec(&cursor).unwrap()).unwrap();
            match &request {
                Request::Camera(open::Request::Zoom {
                    zoom,
                    force,
                    refresh_ui,
                }) => {
                    events.push(json!({"call":"zoom","args":[*zoom,u32::from(*force),u32::from(*refresh_ui)],"state":ui}));
                    if c["mutate"] == true {
                        ui = Ui {
                            flags: 0xa5,
                            zoom: 0x1234,
                            ..Default::default()
                        };
                    }
                }
                Request::RefreshAudio => events.push(json!({"call":"finish","state":ui})),
                _ => panic!("unexpected camera call"),
            }
            cursor.acknowledge(&request).unwrap();
            let before = cursor.clone();
            assert!(cursor.acknowledge(&request).is_err());
            assert_eq!(cursor, before);
        }
        assert!(cursor.complete());
        assert_eq!(json!({"events":events,"state":ui}), row["original"], "{c}");
    }
}

#[cfg(feature = "private-fixtures")]
#[test]
fn rotation_and_floor_setters_match_original_callback_write_order() {
    let f: Value = serde_json::from_str(include_str!("fixtures/camera-controls.json")).unwrap();
    assert_eq!(f["source_sha256"], fixture()["source_sha256"]);
    assert_eq!(f["capture_sha256"], fixture()["capture_sha256"]);
    let rows = f["cases"].as_array().unwrap();
    assert_eq!(rows.len(), 240);
    // Include zero fields, which the backward-compatible save encoding omits.
    let state =
        |u: Ui| json!({"flags":u.flags,"zoom":u.zoom,"rotation":u.rotation,"floor":u.floor});
    for row in rows {
        let c = &row["input"];
        let mut ui = Ui {
            flags: c["flags"].as_u64().unwrap() as u8,
            zoom: 2,
            rotation: 2,
            floor: 1,
        };
        let value = c["value"].as_i64().unwrap() as i32;
        let deferred = c["deferred"].as_u64().unwrap() as u32;
        let mut call = if c["kind"] == "rotation" {
            Zoom::rotation(&mut ui, value as u32, deferred)
        } else {
            Zoom::floor(&mut ui, value, deferred)
        };
        let mut events = vec![];
        while let Some(request) = call.request().cloned() {
            call = serde_json::from_slice(&serde_json::to_vec(&call).unwrap()).unwrap();
            match &request {
                Request::Camera(r) => {
                    let args = match r {
                        open::Request::Rotate {
                            rotation,
                            force,
                            refresh_ui,
                        } => vec![*rotation, u32::from(*force), u32::from(*refresh_ui)],
                        open::Request::Floor { level, force } => {
                            vec![*level as u32, u32::from(*force)]
                        }
                        _ => panic!("wrong camera entry"),
                    };
                    events.push(json!({"call":c["kind"],"args":args,"state":state(ui)}));
                    if c["mutate"] == true {
                        ui.flags = 0xa0;
                        ui.rotation = 7;
                        ui.floor = 9;
                    }
                }
                Request::RefreshControls => {
                    events.push(json!({"call":"controls","state":state(ui)}))
                }
                _ => panic!("unexpected setter callback"),
            }
            call.acknowledge_with_ui(&request, &mut ui).unwrap();
            let previous = (call.clone(), ui);
            assert!(call.acknowledge_with_ui(&request, &mut ui).is_err());
            assert_eq!((call.clone(), ui), previous);
        }
        assert_eq!(
            json!({"events":events,"state":state(ui)}),
            row["original"],
            "{c}"
        );
    }
}
#[test]
fn sprite_depth_table_matches_original_x87_and_crt_words() {
    let f = fixture();
    let rows = f["depths"].as_array().unwrap();
    assert_eq!(rows.len(), 215);
    for row in rows {
        let bits = row["bits"].as_u64().unwrap() as u32;
        assert_eq!(
            json!(camera_command::depth_table(bits).as_slice()),
            row["original"],
            "scale bits {bits:#010x}"
        );
    }
}
