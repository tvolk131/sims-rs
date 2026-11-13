//!48bdbd and4929e7/4929f1/4929fb: accumulate a signed adjustment, temporarily
//!select category6/7, flush through46d91c, then clear and restore the category.
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
impl crate::Simulation {
    /// The direct engine call shares the clock's live daily statistics block.
    /// Synchronize its Rust projection before any callback or saved boundary.
    pub fn advance_object_budget(
        &mut self,
        call: &mut sims_vm::budget::Application,
    ) -> Result<Option<i32>, String> {
        let result = call.advance(&mut self.vm.world);
        self.publish_budget_statistics();
        result
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    pub pending: i32,
    pub category: i32,
}
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Budgets(pub BTreeMap<u32, State>);
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Apply {
    pub category: i32,
    pub amount: i32,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Call {
    owner: u32,
    amount: i32,
    special: u8,
    stage: u8,
    old_category: i32,
    pending: Option<Apply>,
}
impl Call {
    pub fn new(owner: u32, amount: i32, special: u8) -> Self {
        Self {
            owner,
            amount,
            special,
            stage: 0,
            old_category: 0,
            pending: None,
        }
    }
    pub fn complete(&self) -> bool {
        self.stage == 2 && self.pending.is_none()
    }
    pub fn acknowledge(&mut self, r: Apply) -> Result<(), String> {
        if self.stage != 1 || self.pending != Some(r) {
            return Err("budget reply differs or has already returned".into());
        }
        self.pending = None;
        Ok(())
    }
    pub fn advance(&mut self, b: &mut Budgets) -> Result<Option<Apply>, String> {
        if self.stage > 2 || (self.pending.is_some() && self.stage != 1) {
            return Err("invalid saved budget stage".into());
        }
        if self.pending.is_some() {
            return Ok(self.pending);
        }
        if self.complete() {
            return Ok(None);
        }
        let s = b.0.get_mut(&self.owner).ok_or("budget owner absent")?;
        if self.stage == 0 {
            self.old_category = s.category;
            s.category = 6 + i32::from(self.special != 0);
            //NEG/SUB in the negative branch has the same wrapping result,
            //including INT_MIN. Preserve pre-existing pending adjustments.
            s.pending = s.pending.wrapping_add(self.amount);
            self.pending = Some(Apply {
                category: s.category,
                amount: s.pending,
            });
            self.stage = 1;
        } else {
            s.pending = 0;
            s.category = self.old_category;
            self.stage = 2;
        }
        Ok(self.pending)
    }
}
