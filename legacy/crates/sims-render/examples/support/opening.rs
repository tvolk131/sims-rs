//! Actual replacement-client owners used by the interactive viewer. No
//! captured object ID, indicator position or desired result is injected.
use anyhow::{Context, Result, ensure};
use sims_render::{
    camera_device::Device,
    dialog::client::{Client, Inputs},
    original_house::OriginalHouse,
};
use sims_runtime::original_session::{Session, ui_wait};

pub fn bind(session: &mut Session, device: &mut Device) -> Result<Client> {
    let mut client = Client::new(session)?;
    let size = session
        .simulation
        .architecture
        .as_ref()
        .and_then(|a| a.original.as_ref())
        .context("opening lot absent")?
        .size as i32;
    let inputs = Inputs::camera(OriginalHouse::native_camera(session)?, size)?;
    client.bind_resident_buttons(session, inputs)?;
    for _ in 0..10000 {
        if !client.ui_lookup_pending() {
            break;
        }
        client.advance_ui_lookup(session, 256)?;
    }
    ensure!(!client.ui_lookup_pending(), "opening portrait budget");
    device.publish_opening_household(session, 0)?;
    Ok(client)
}

pub fn poll(session: &mut Session, client: &Client) -> Result<()> {
    //The interactive viewer runs4764fe before its lot updates. This bounded
    //trace supplies its real committed-control input and stops at any widget
    //observation whose actual owner is not serviced by this diagnostic.
    let focused = client.buttons().and_then(|b| b.focused_resource());
    poll_resource(session, focused)
}

/// Explicit application observation for a recorded/diagnostic UI poll. The
/// shared 4764fe owner performs publication and any named callbacks normally.
pub fn poll_resource(session: &mut Session, focused: Option<i32>) -> Result<()> {
    session.begin_ui_wait(focused)?;
    for _ in 0..30000 {
        match session.advance_ui_wait(256)? {
            ui_wait::Progress::Complete => return Ok(()),
            ui_wait::Progress::Pending => (),
            p => anyhow::bail!("application poll requires widget input: {p:?}"),
        }
    }
    anyhow::bail!("application poll budget")
}
