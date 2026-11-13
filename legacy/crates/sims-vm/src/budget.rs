//! Lot budget services from 0x44eade, 0x46d8c9 and 0x46d91c.
use crate::{Program, Thread, Vm, World};
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct State {
    pub balance: i32,
    /// Native +0x184..+0x1a4. The first eight are the clock's current daily block;
    /// category eight lies just beyond those rolling statistics.
    pub expenses: [i32; 9],
    /// Ordered native signal 0xda payloads for a presentation/world consumer.
    pub notifications: Vec<i32>,
}
/// Direct46d91c, distinct from the bytecode primitive's affordability wrapper.
/// DA is a synchronous child: consumers must finish it before acknowledging.
/// This path never adds a second copy to the primitive's notification queue.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Application {
    category: i32,
    amount: i32,
    stage: ApplicationStage,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
enum ApplicationStage {
    Entry,
    Broadcast(i32),
    Complete,
}
impl Application {
    pub fn new(category: i32, amount: i32) -> Self {
        Self {
            category,
            amount,
            stage: ApplicationStage::Entry,
        }
    }
    pub fn input(&self) -> (i32, i32) {
        (self.category, self.amount)
    }
    pub fn pending(&self) -> Option<i32> {
        match self.stage {
            ApplicationStage::Broadcast(n) => Some(n),
            _ => None,
        }
    }
    pub fn complete(&self) -> bool {
        self.stage == ApplicationStage::Complete
    }
    pub fn advance(&mut self, world: &mut World) -> Result<Option<i32>, String> {
        if self.stage != ApplicationStage::Entry {
            return Ok(self.pending());
        }
        //The original early return does not touch funds or the category.
        if *world
            .globals
            .get(&32)
            .ok_or("lot budget mode unavailable")?
            == 1
        {
            self.stage = ApplicationStage::Complete;
            return Ok(None);
        }
        //Native callers supply0..8. Other values address memory outside this
        //projection; diagnose them before entering rather than invent storage.
        let category = usize::try_from(self.category)
            .ok()
            .filter(|&n| n < 9)
            .ok_or("direct budget category outside retained storage")?;
        let state = world
            .budget
            .as_mut()
            .ok_or("lot budget storage unavailable")?;
        state.balance = state.balance.wrapping_sub(self.amount);
        state.expenses[category] = state.expenses[category].wrapping_add(self.amount);
        let balance = state.balance;
        //Signed IDIV truncates toward zero. Preserve the high-word store first.
        world.globals.insert(14, (balance / 10000) as i16);
        world.globals.insert(13, (balance % 10000) as i16);
        self.stage = ApplicationStage::Broadcast(balance);
        Ok(Some(balance))
    }
    pub fn acknowledge(&mut self, balance: i32) -> Result<(), String> {
        if self.pending() != Some(balance) {
            return Err("budget broadcast reply differs".into());
        }
        //Callbacks may change funds again. No reread or replay after returning.
        self.stage = ApplicationStage::Complete;
        Ok(())
    }
}
impl Vm {
    pub(crate) fn change_budget(
        &mut self,
        p: &Program,
        t: &mut Thread,
        b: &[u8; 8],
    ) -> Result<(bool, String), String> {
        let category = usize::from(b[6]);
        if category > 8 {
            return Err("invalid budget category (native error 0x38)".into());
        }
        let scope = match b[0] {
            0 => 7,
            1 => 9,
            2 => 25,
            _ => b[1],
        };
        let mut cost = i32::from(self.read(p, t, scope, i16::from_le_bytes([b[2], b[3]]))?);
        let mode = *self
            .world
            .globals
            .get(&32)
            .ok_or("lot budget mode unavailable")?;
        let state = self
            .world
            .budget
            .as_mut()
            .ok_or("lot budget storage unavailable")?;
        let available = if mode == 1 { 999999 } else { state.balance };
        if b[4] & 2 != 0 {
            cost = -cost;
        }
        let affordable = cost < 0 || cost <= available;
        if affordable && b[4] & 1 == 0 && mode != 1 {
            if state.notifications.len() >= 4096 {
                return Err("budget notification queue is full; drain before continuing".into());
            }
            state.balance = state.balance.wrapping_sub(cost);
            state.expenses[category] = state.expenses[category].wrapping_add(cost);
            self.world
                .globals
                .insert(13, (state.balance % 10000) as i16);
            self.world
                .globals
                .insert(14, (state.balance / 10000) as i16);
            state.notifications.push(state.balance);
        }
        Ok((
            affordable,
            format!(
                "native budget cost {cost}, category {category}, check {}, allowed {affordable}",
                b[4] & 1 != 0
            ),
        ))
    }
}
