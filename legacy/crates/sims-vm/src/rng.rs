//! Two distinct native random streams. A caller must retain each state separately.
use crate::Vm;
use serde::{Deserialize, Serialize};

/// The suspended 46d345 stack local around46d553. During this call rng_state
/// is the working lot stream; simulator+ac still contains its entry seed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LotScope {
    tick: u64,
    saved_global: u32,
    lot_at_entry: u32,
}
impl LotScope {
    pub fn validate(&self, vm: &Vm) -> Result<(), String> {
        if self.tick != vm.tick || vm.lot_rng_state != Some(self.lot_at_entry) {
            return Err("suspended lot RNG scope lost its tick or stored seed".into());
        }
        Ok(())
    }
}
impl Vm {
    ///46d345 before46d553. Budgets and synchronous host yields do not leave
    ///this scope; every existing native RNG consumer uses the working stream.
    pub fn begin_lot_rng_scope(&mut self) -> Result<(), String> {
        if self.lot_rng_scope.is_some() {
            return Err("lot RNG scope already active".into());
        }
        let lot = self.lot_rng_state.ok_or("native lot RNG seed absent")?;
        self.lot_rng_scope = Some(LotScope {
            tick: self.tick,
            saved_global: self.rng_state,
            lot_at_entry: lot,
        });
        self.rng_state = lot;
        Ok(())
    }
    ///46d345 immediately after46d553 returns, including false/partial returns.
    ///Restore before calendar/world-update/notification work, not at tick end.
    pub fn end_lot_rng_scope(&mut self) -> Result<(), String> {
        let scope = self.lot_rng_scope.as_ref().ok_or("lot RNG scope absent")?;
        scope.validate(self)?;
        let global = scope.saved_global;
        self.lot_rng_state = Some(self.rng_state);
        self.rng_state = global;
        self.lot_rng_scope = None;
        Ok(())
    }
}

/// Exact 0x0046f457 stream: retain low 32 bits, return bits 16..47 of
/// the untruncated product. In particular, the output is not masked to 15/16 bits.
pub fn next(state: &mut u32) -> u32 {
    let product = u64::from(*state) * 0x41c64e6d + 0x3039;
    *state = product as u32;
    (product >> 16) as u32
}

/// CRT `_rand` at 0x636c74, used by person default-trait initialization.
/// Its native state lives in per-thread CRT storage, not the lot RNG in SIMI.
pub fn next_crt(state: &mut u32) -> u32 {
    *state = state.wrapping_mul(0x343fd).wrapping_add(0x269ec3);
    (*state >> 16) & 0x7fff
}

/// Modern SIMI RNG load block, 0x46d09c..0x46d0e8. The first serialized
/// word is simulator-owned; the second seeds `next`. Nonresidential loading
/// takes two consecutive timer readings, one for each stream, in this order.
/// The host supplies actual readings; this function never consults wall time.
pub fn load_saved(
    lot: u32,
    global: u32,
    residential: bool,
    timer_readings: Option<[u32; 2]>,
) -> Result<[u32; 2], String> {
    if residential {
        Ok([lot, global])
    } else {
        timer_readings
            .ok_or_else(|| "nonresidential RNG loading requires two timer readings".into())
    }
}
