//! Synchronous 4e77c6 preparation: forced 58a178 selection, then 4e810d's
//! exact HEAD bindings. Drawing, camera acquisition and pixel ownership follow
//! this call; a dirty cached portrait cannot be replaced by a saved BMP here.
use super::*;

impl<P: Presentation + Clone> Host<P> {
    /// Shared native preparation plus per-binding rest-pose drawing/restoration.
    /// The presenter supplies its actual matrices, modes and graphics backend;
    /// this does not substitute diagnostic camera or lighting inputs.
    pub fn draw_portrait<R: crate::character_portrait::Renderer + Clone>(
        &mut self,
        simulation: &mut Simulation,
        program: &Program,
        id: ObjectId,
        matrix: sims_sim::pose::model::Matrix,
        renderer: &mut R,
    ) -> Result<sims_sim::pose::portrait::draw::Outcome> {
        self.prepare_portrait(simulation, program, id)?;
        let assets = self
            .characters
            .get_mut(&id)
            .context("portrait assets absent")?;
        let resident = simulation
            .characters
            .get(&id)
            .context("portrait resident absent")?;
        crate::character_portrait::render(
            &mut assets.geometry,
            &assets.avatar,
            resident,
            matrix,
            renderer,
        )
    }
    /// Native portrait selection is forced, independently of ordinary draw.
    pub fn prepare_portrait(
        &mut self,
        simulation: &mut Simulation,
        program: &Program,
        id: ObjectId,
    ) -> Result<Vec<(usize, usize)>> {
        self.prepare_appearance(simulation, program, id, true)
    }
}
