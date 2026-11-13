//! House usage samples (HOUS 1), separate from SIMI's budget history.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    /// Native +38..64: five history pairs, then current [total, active action].
    pub pairs: [[u32; 2]; 6],
}
impl State {
    /// 416568 reads active before total for each pair, including the current day.
    pub fn from_saved_pairs(pairs: [[i32; 2]; 6]) -> Self {
        Self {
            pairs: pairs.map(|[active, total]| [total as u32, active as u32]),
        }
    }
    /// 4165e3. Preserve update order and 32-bit wrapping.
    pub fn sample(&mut self, has_action: bool) {
        if has_action {
            self.pairs[5][1] = self.pairs[5][1].wrapping_add(1);
        }
        self.pairs[5][0] = self.pairs[5][0].wrapping_add(1);
    }
    /// 41637f's DC branch, before its separate valuation update. The native
    /// forward copy propagates old history[0] through all four following pairs.
    pub fn new_day(&mut self) {
        for i in 1..5 {
            self.pairs[i] = self.pairs[i - 1];
        }
        self.pairs[0] = self.pairs[5];
        self.pairs[5] = [0; 2];
    }
}
