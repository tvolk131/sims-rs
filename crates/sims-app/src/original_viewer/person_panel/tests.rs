use super::*;

fn panel() -> Result<Panel> {
    Ok(serde_json::from_value(serde_json::json!({
        "tick": 0, "person": { "object": 196, "guid": 42, "cache_index": 2,
            "neighbor": 48, "name": "Bob", "values": [0,0,0,0,0,0,0,0],
            "details": { "personality": [100,200,300,400,500], "skills": [100,200,300,400,500,600],
                "relationships": [], "job": null } },
        "state": 1, "shown": true, "category": "Personality", "popup": {"Personality":0},
        "invalidations": 0,
        "texts": { "personality": vec!["Trait";36], "relationships": vec!["Relationship";3],
            "skills": vec!["Skill";7], "details": vec!["Description";24] }
    }))?)
}

#[test]
fn tutorial_panel_input_rejects_stale_identity_modal_and_hidden_controls() -> Result<()> {
    let panel = panel()?;
    let identity = panel.person().unwrap().identity();
    let message = Message::PersonDetails(identity, Detail::Personality(0));
    ensure!(available(&panel, Some(identity), &message, false, true));
    ensure!(!available(&panel, Some(identity), &message, true, true));
    ensure!(!available(&panel, Some(identity), &message, false, false));
    ensure!(!available(&panel, None, &message, false, true));
    let changed = Identity { object: 137, guid: 999, ..identity };
    ensure!(!available(&panel, Some(changed), &message, false, true));
    let close = Message::ClosePersonDetails(identity, Popup::Detail(Detail::Personality(0)));
    ensure!(available(&panel, Some(identity), &close, false, false));
    let stale_close = Message::ClosePersonDetails(identity, Popup::Detail(Detail::Personality(1)));
    ensure!(!available(&panel, Some(identity), &stale_close, false, true));
    Ok(())
}

#[test]
fn tutorial_panel_categories_emit_real_resource_requests_and_visible_popup() -> Result<()> {
    let panel = panel()?;
    let identity = panel.person().unwrap().identity();
    for category in [Category::Personality, Category::Relationships, Category::Job] {
        let message = category_message(identity, category);
        ensure!(request(&message) == Some((identity, Some(category))));
        ensure!(available(&panel, Some(identity), &message, false, true));
    }
    for category in Category::ALL {
        ensure!(panel.category_selected(category) == (category == Category::Personality));
        ensure!(panel.observation(&sims_vm::ui_owner::wait::Condition::Control(category.control()))
            == Some(sims_vm::ui_owner::wait::Observation::Control {image:Some(category.control()),
                state:u32::from(category == Category::Personality)}));
        ensure!(hud_position(category.control()).is_some());
    }
    ensure!(popup(&panel).is_some());
    let _ = sidebar(&panel, true, Some(4501));
    Ok(())
}

#[test]
fn tutorial_panel_narrow_categories_wrap_and_click_actual_controls() -> Result<()> {
    let panel = panel()?;
    let identity = panel.person().unwrap().identity();
    let mut input = headless_input::Input::new()?;
    for width in [230, 280, 340] {
        let display = [width, 400];
        let observation = input.element_observation(sidebar(&panel, true, None), display)?;
        let mut positions = Vec::new();
        for category in Category::ALL {
            let bounds = observation.texts.iter().find(|entry| entry.text == category.label())
                .context("category label absent")?.bounds;
            ensure!(bounds.x >= 0. && bounds.x + bounds.width <= width as f32);
            positions.push(bounds.center());
            let events = click(bounds.center());
            let batch = input.element_batch(sidebar(&panel, true, None), display, &events);
            ensure!(batch.messages.len() == 1);
            ensure!(control_resource(&batch.messages[0]) == Some(category.control()));
            if category != Category::Needs {
                ensure!(request(&batch.messages[0]) == Some((identity, Some(category))));
            }
            let batch = input.element_batch(sidebar(&panel, false, None), display, &events);
            ensure!(batch.messages.is_empty());
        }
        ensure!(positions[0].y == positions[1].y && positions[2].y == positions[3].y
            && positions[2].y > positions[0].y);
    }
    ensure!(control_resource(&Message::PersonDetails(identity, Detail::Personality(0))).is_none());
    Ok(())
}

#[test]
fn tutorial_panel_highlighted_relationship_click_retains_directed_target_identity() -> Result<()> {
    let panel: Panel = serde_json::from_value(serde_json::json!({
        "tick": 0, "person": { "object": 105, "guid": 850010198, "cache_index": 256,
            "neighbor": 48, "name": "Bob", "values": [0,0,0,0,0,0,0,0],
            "details": { "personality": null, "skills": null, "job": null,
                "relationships": [{ "neighbor": 47, "guid": 844195932, "cache_index": 266,
                    "name": "Betty", "daily": 55, "lifetime": 12, "flags": [false,false], "highlighted": true }] } },
        "state": 1, "shown": true, "category": "Relationships", "popup": null, "invalidations": 0,
        "texts": { "personality": vec!["Trait";36], "relationships": vec!["Relationship";3],
            "skills": vec!["Skill";7], "details": vec!["Description";24] }
    }))?;
    let mut input = headless_input::Input::new()?;
    let observation = input.element_observation(content(&panel, true), [280, 200])?;
    let target = observation.texts.iter().find(|entry| entry.text == "▶ Betty · daily 55 / lifetime 12")
        .context("actual highlighted relationship target absent")?;
    let batch = input.element_batch(content(&panel, true), [280, 200], &click(target.bounds.center()));
    ensure!(batch.messages.len() == 1 && matches!(batch.messages[0],
        Message::PersonDetails(identity, Detail::Relationship {neighbor:47, guid:844195932, cache_index:266})
            if identity == panel.person().unwrap().identity()));
    ensure!(control_resource(&batch.messages[0]).is_none());
    Ok(())
}

fn click(position: iced::Point) -> [iced::Event; 3] {
    [iced::Event::Mouse(iced::mouse::Event::CursorMoved { position }),
        iced::Event::Mouse(iced::mouse::Event::ButtonPressed(iced::mouse::Button::Left)),
        iced::Event::Mouse(iced::mouse::Event::ButtonReleased(iced::mouse::Button::Left))]
}
