use super::memory::Identity;
use sims_sim::saved_matrix::Stop;
use sims_sim::{native_application::InitializedSection, native_world_children::Owner};
use std::sync::{Arc, Mutex};

pub(super) struct Section {
    pub address: u64,
    pub bytes: u32,
    pub lock: Arc<Mutex<()>>,
    pub debug_info: Option<Owner>,
    pub parent: Option<(Identity, u64, u32)>,
}

impl Section {
    pub fn create(address: u64, bytes: u32) -> Result<Self, Stop> {
        if address != 0x6fe578 || bytes != 24 {
            return Err(Stop::Unsupported(
                "unqualified resource manager section request".into(),
            ));
        }
        Ok(Self {
            address,
            bytes,
            lock: Arc::new(Mutex::new(())),
            debug_info: None,
            parent: None,
        })
    }

    /// The caller has already validated the initialized image. This lock owns
    /// Rust synchronization storage; native enter/leave effects are not implied.
    pub fn application(reply: &InitializedSection, parent: u64, heap: Identity) -> Self {
        Self {
            address: reply.region.token,
            bytes: 24,
            lock: Arc::new(Mutex::new(())),
            debug_info: reply.debug_info.clone(),
            parent: Some((heap, parent, 324)),
        }
    }
}
