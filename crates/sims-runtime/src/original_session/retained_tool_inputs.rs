//! Process-owned tool inputs, distinct from fresh frontend driver construction.
//! Handles are portable identities; class words identify the captured code ABI.
use super::Session;
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use sims_vm::{ObjectId, interface::tool};
use std::collections::BTreeSet;
pub mod stock;

/// cTool scalar storage, copied from the actual instance. Offset names avoid
/// guessing meanings for the eight base words and+24. +28 gates492c14's tile
/// overlay; it is1 for the captured Wallpaper tool despite being inactive.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Base {
    pub words_04_20: [u32; 8],
    pub word_24: u32,
    pub tile_overlay: u8,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Slot {
    pub handle: u32,
    pub class: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base: Option<Base>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Picker {
    pub handle: u32,
    pub mode: Option<i32>,
    /// Native+68. This is also a48cba9 click-dispatch gate. Constructor5
    /// cannot replace an observed live value (28 at the captured boundary).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<i32>,
    /// Native+100 cache; it can outlive its object. Only the original actor
    /// gate refreshes it, not generic selection or a restore operation.
    pub actor: Option<ObjectId>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    slots: [Slot; 12],
    selected: tool::Fields,
    picker: Picker,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    stock: Option<stock::Stock>,
}
impl State {
    pub(super) fn from_completed_inputs(
        slots: [Slot; 12],
        selected: tool::Fields,
        picker: Picker,
    ) -> Result<Self> {
        let state = Self {
            slots,
            selected,
            picker,
            stock: None,
        };
        state.validate_fields()?;
        Ok(state)
    }
    pub fn slots(&self) -> &[Slot; 12] {
        &self.slots
    }
    pub(super) fn with_bases(mut self, bases: [Base; 12]) -> Result<Self> {
        ensure!(
            self.slots.iter().all(|s| s.base.is_none()),
            "stock tool bases already bound"
        );
        for (slot, base) in self.slots.iter_mut().zip(bases) {
            slot.base = Some(base);
        }
        self.validate_fields()?;
        Ok(self)
    }
    pub fn selected(&self) -> &tool::Fields {
        &self.selected
    }
    pub fn picker(&self) -> &Picker {
        &self.picker
    }
    pub fn stock(&self) -> Option<&stock::Stock> {
        self.stock.as_ref()
    }
    pub(super) fn with_stock(mut self, stock: stock::Stock) -> Result<Self> {
        ensure!(
            self.stock.is_none() && self.slots.iter().all(|s| s.base.is_some()),
            "stock configuration writer overlaps or bases are unbound"
        );
        stock.validate()?;
        self.stock = Some(stock);
        self.validate_fields()?;
        Ok(self)
    }
    pub fn is_picker(&self, id: u32) -> bool {
        id == self.picker.handle
    }
    pub fn selected_picker_or_null(&self) -> bool {
        self.selected.active.is_none_or(|id| self.is_picker(id))
    }
    pub(super) fn set_actor(&mut self, actor: Option<ObjectId>) {
        self.picker.actor = actor;
    }
    pub(super) fn activate_picker(&mut self, id: u32, argument: u32) -> Result<()> {
        ensure!(
            self.is_picker(id),
            "retained tool activation receiver is not the captured Picker"
        );
        tool::activate_picker(&mut self.picker.mode, argument);
        Ok(())
    }
    pub(super) fn validate_fields(&self) -> Result<()> {
        if let Some(stock) = &self.stock {
            stock.validate()?;
            ensure!(
                self.slots.iter().all(|s| s.base.is_some()),
                "retained stock configuration lost its native bases"
            );
        }
        ensure!(
            self.slots.iter().all(|s| s.base.is_none())
                || self.slots.iter().all(|s| s.base.is_some()),
            "partially bound stock tool bases"
        );
        let handles: BTreeSet<_> = self.slots.iter().map(|s| s.handle).collect();
        ensure!(
            handles.len() == 12 && !handles.contains(&0) && self.slots.iter().all(|s| s.class != 0),
            "retained stock-tool identities are incomplete or repeated"
        );
        ensure!(
            self.slots[6].handle == self.picker.handle
                && self.slots[6].class == 0x667f1c
                && self.slots[7].class == 0x667cd0,
            "retained Picker/Move slot classes differ"
        );
        ensure!(
            self.selected.active.is_none_or(|id| handles.contains(&id))
                && self.picker.actor.is_none_or(|id| id > 0),
            "retained tool selection or cached actor is invalid"
        );
        Ok(())
    }
    pub(super) fn validate(&self, session: &Session) -> Result<()> {
        self.validate_fields()?;
        ensure!(
            session.host.presentation.tool_selection.is_none(),
            "native and constructed indexed-tool writers overlap"
        );
        ensure!(
            session.host.presentation.transport.is_some(),
            "retained tool inputs lost CPState"
        );
        let current = session
            .host
            .presentation
            .current_tool
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("retained tool inputs lost global current-tool owner"))?
            .current();
        ensure!(
            current.is_none_or(|id| self.slots.iter().any(|s| s.handle == id)),
            "global current tool is outside the retained stock registry"
        );
        Ok(())
    }
}
impl Session {
    ///48cc2e/48cc3c stores the getter result into Picker+100 before the null
    ///test. Existing authored sessions have no imported cache and stay so.
    pub(super) fn picker_input_actor(&mut self) -> Result<Option<ObjectId>> {
        let actor = self
            .simulation
            .native_selected_person()
            .map_err(anyhow::Error::msg)?;
        if let Some(inputs) = &mut self.host.presentation.retained_tool_inputs {
            inputs.set_actor(actor);
        }
        Ok(actor)
    }
}
