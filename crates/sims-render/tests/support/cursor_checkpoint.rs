use anyhow::{Context, Result};
use sims_content::Index;
use sims_runtime::original_session::{Saved, Session};
use sims_vm::ObjectId;
use std::path::Path;

pub fn read_camera(path: &Path) -> Result<sims_render::camera_device::Saved> {
    let wrapper: serde_json::Value = serde_json::from_slice(&std::fs::read(path)?)?;
    Ok(serde_json::from_value(wrapper["camera_device"].clone())?)
}

pub fn object_for_handle(session: &Session, handle: u32) -> Option<ObjectId> {
    session
        .simulation
        .vm
        .world
        .scheduler_storage
        .as_ref()
        .unwrap()
        .object_for_handle(handle)
}

pub fn focused_cursor(session: &Session) -> Result<(u32, ObjectId)> {
    let focus = session
        .host
        .presentation
        .object_focus
        .context("cursor focus absent")?;
    Ok((focus, object_for_handle(session, focus).unwrap()))
}

pub fn underlying_object(session: &Session, cursor: ObjectId) -> ObjectId {
    let handle = session.simulation.vm.world.objects[&cursor]
        .cursor
        .as_ref()
        .unwrap()
        .underlying
        .unwrap();
    object_for_handle(session, handle).unwrap()
}

pub fn restore_json(index: &Index, snapshot: &Saved) -> Result<Session> {
    Session::restore(
        index,
        serde_json::from_slice(&serde_json::to_vec(snapshot)?)?,
    )
}
