//! Passive native route cleanup observations. No route success is inferred.
use crate::Simulation;
pub use sims_vm::observations::NativeRouteTerminalEvent;
use sims_vm::observations::ObservationBuffer;

impl Simulation {
    /// Starts independent bounded action and native-route observation buffers.
    /// Both are disabled and emptied when this simulation is cloned or saved.
    pub fn enable_terminal_observations(&mut self, capacity: usize) {
        self.vm.enable_terminal_observations(capacity);
    }
    pub fn disable_terminal_observations(&mut self) {
        self.vm.disable_terminal_observations();
    }
    pub fn route_terminal_observations(&self) -> &ObservationBuffer<NativeRouteTerminalEvent> {
        self.vm.route_terminal_observations()
    }
    pub fn drain_route_terminal_observations(&mut self) -> Vec<NativeRouteTerminalEvent> {
        self.vm.drain_route_terminal_observations()
    }
}
