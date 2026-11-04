//! Fresh replacement application owners. These are logical IDs for our own
//! tools/widgets, never inferred Windows addresses or imported continuations.
use super::*;

pub(super) fn bind(
    session: &mut Session,
    device: &mut CameraDevice,
    client: &mut DialogClient,
) -> Result<()> {
    use sims_render::camera_device::{
        mode::Window,
        panels::{Owners, Panel},
    };
    use sims_runtime::original_session::publication::{Context, Progress};
    use sims_runtime::original_session::{
        move_tool,
        tool_selection::{self, Receiver},
    };
    use sims_sim::object_catalog::move_tool::{Groups, cleanup};

    ensure!(
        session.simulation.native_scheduler.is_some(),
        "Buy requires original partial scheduling"
    );
    session.bind_person_publication(0)?;
    let mut catalog = sims_content::object_catalog::Catalog::household(
        Arc::new(session.index().clone()),
        &session.house.neighborhood_source,
        Default::default(),
    )?;
    let mut history = move_tool::History::new(1);
    history.retain_catalog(&mut catalog)?;
    history
        .store
        .construct_manager(5001)
        .map_err(anyhow::Error::msg)?;
    let mut groups = Groups::default();
    catalog
        .populate_move_tool_groups(&mut groups)
        .map_err(anyhow::Error::msg)?;
    let mut moving = move_tool::State::new(
        cleanup::State::constructor_fields(5001),
        cleanup::Inputs {
            registry: 5002,
            window: 0,
            key_function: 0,
            control_state: 0,
        },
        history,
    );
    moving.retain_activation_groups(groups)?;
    session.bind_move_tool(moving)?;
    let mut slots = [None; 12];
    slots[6] = Some(6000);
    slots[7] = Some(7000);
    let mut tools = tool_selection::State::new(
        slots,
        [
            (6000, Receiver::Picker { mode: Some(0) }),
            (7000, Receiver::Move),
        ]
        .into(),
        sims_vm::interface::tool::Fields {
            active: Some(6000),
            argument: 0,
        },
    );
    tools.bind_mode_context(tool_selection::mode::Context::new(8000).with_history_manager(5001))?;
    session.bind_tool_selection(tools)?;
    session.bind_current_tools(Some(6000))?;
    let controls = &session
        .host
        .presentation
        .transport
        .as_ref()
        .context("transport absent")?
        .controls;
    device.bind_mode_window(
        session,
        Window::new(9000, Some(9001), 80, controls.paused, 0),
    )?;
    if session
        .host
        .presentation
        .presenter
        .as_ref()
        .and_then(|p| p.camera.as_ref())
        .is_some_and(|c| c.settings.roof.is_none())
    {
        device.bind_hidden_roof(session)?;
    }
    // 4ce81c samples the actual view with this precedence. A fresh Live view
    // normally has cutaway enabled; publishing "walls up" would change it.
    let presenter = session
        .host
        .presentation
        .presenter
        .as_ref()
        .context("view absent")?;
    let walls = if presenter.view.floor.all_walls != 0 {
        2
    } else if presenter.view.floor.visibility.enabled != 0 {
        4
    } else if presenter
        .camera
        .as_ref()
        .and_then(|c| c.settings.roof)
        .context("roof absent")?
        != 0
    {
        8
    } else {
        1
    };
    session.begin_interface_publication(
        Context {
            panel_owner: Some(10000),
            closing: 0,
            walls,
            tool_gate: 0,
            current_tool: Some(6000),
            selected_person_index: 0,
        },
        0,
    )?;
    for _ in 0..10000 {
        match device.advance_publication(session, 256)? {
            Progress::Complete => {
                device.bind_interface_properties(session, 30000, 30001)?;
                session.bind_property_listener(8)?;
                session.bind_frame_transport()?;
                return Ok(());
            }
            Progress::Pending => (),
            Progress::Panels(_) if device.panel_widgets().is_none() => {
                device.bind_panel_widgets(
                    session,
                    Owners {
                        panel: 10000,
                        state: 10001,
                        controls: 10002,
                        lot: 10003,
                        manager: 10004,
                        panels: std::array::from_fn(|i| 10010 + i as u32),
                        widgets: std::array::from_fn(|i| 10100 + i as u32),
                        pause: 10200,
                    },
                    1,
                )?;
                device.bind_panel_presentation(session, Panel::People)?;
                device.bind_panel_text(session)?;
                device.bind_panel_access(session)?;
                device.bind_panel_catalog(session, 8)?;
            }
            Progress::Panels(_) if device.publish_replacement_people(session, client)? => (),
            other => anyhow::bail!(
                "play controls require unsupported startup service: {other:?}, {:?}",
                device.panel_widgets().and_then(|w| w.boundary())
            ),
        }
    }
    anyhow::bail!("play controls startup budget exceeded")
}

#[cfg(feature = "private-fixtures")]
#[cfg(test)]
#[path = "original_viewer_play/private_tests.rs"]
#[rustfmt::skip]
mod tests;
