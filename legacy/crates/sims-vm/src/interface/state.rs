//! Shared CPState fields. Mode and Buy kernels borrow projections of this
//! owner; they must not retain independent copies across callbacks.
use super::{buy, mode_change};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    pub mode: i32,
    pub paused: u8,
    pub speed: i32,
    ///CPState+1ec. Historical snapshots retain absence; constructor writes -1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected_person_index: Option<i32>,
    /// CPState+200/+210. Historical saves do not invent these process timers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speed_timers: Option<super::speed::Timers>,
    /// CPState+10/+14. A historical snapshot does not infer these values.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub publication: Option<super::publication::Fields>,
    pub build_tool: i32,
    pub buy_tool: i32,
    /// Owns the shared dirty word and catalog selectors as well as Buy fields.
    pub buy: buy::Fields,
}
impl State {
    /// The retained scalar/cache subset of4ce0b9. The original speed default
    /// comes from4425a2 and must be supplied by its owner. No window/tool/list
    /// constructors are claimed by this data initializer.
    pub fn constructed(speed: i32) -> Self {
        Self {
            mode: 3,
            paused: 0,
            speed,
            selected_person_index: Some(-1),
            speed_timers: Some(Default::default()),
            publication: Some(Default::default()),
            build_tool: 11,
            buy_tool: 3,
            buy: buy::Fields {
                dirty: u32::MAX,
                catalog: 1,
                subcatalog: 0,
                indices: [[0; 2]; 7],
                cells: vec![0; 64],
                selected_table: 0,
                selected_object: 0,
                selection_aux: u32::MAX,
            },
        }
    }
    pub fn validate(&self) -> Result<(), String> {
        self.buy.validate()
    }
    pub fn mark_dirty(&mut self, bits: u32) {
        self.buy.dirty |= bits;
    }
    /// View+4c0 and process6e57d0 remain external owned bytes.
    pub fn mode_fields(&self, camera_enabled: u8, notify_pending: u8) -> mode_change::Fields {
        mode_change::Fields {
            mode: self.mode,
            dirty: self.buy.dirty,
            build_tool: self.build_tool,
            buy_tool: self.buy_tool,
            catalog: self.buy.catalog,
            subcatalog: self.buy.subcatalog,
            camera_enabled,
            notify_pending,
        }
    }
    /// Publish immediately after one advance, before servicing its request.
    /// Return the two external bytes for publication to their actual owners.
    pub fn publish_mode_fields(&mut self, fields: mode_change::Fields) -> (u8, u8) {
        self.mode = fields.mode;
        self.buy.dirty = fields.dirty;
        self.build_tool = fields.build_tool;
        self.buy_tool = fields.buy_tool;
        self.buy.catalog = fields.catalog;
        self.buy.subcatalog = fields.subcatalog;
        (fields.camera_enabled, fields.notify_pending)
    }
}
