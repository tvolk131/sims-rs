use sims_sim::edit_history::snapshot::{
    self, CaptureView, Registry, lifecycle, live::View, placement,
};

pub struct CaptureOnly<'a>(pub View<'a>);

impl Registry for CaptureOnly<'_> {
    fn lookup(&self, id: i16) -> Result<Option<u32>, String> {
        Registry::lookup(&self.0, id)
    }
    fn on_lot(&mut self, id: u32) -> Result<bool, String> {
        Registry::on_lot(&mut self.0, id)
    }
}
impl CaptureView for CaptureOnly<'_> {
    fn normalized_master(&self, object: u32) -> Result<Option<u32>, String> {
        CaptureView::normalized_master(&self.0, object)
    }
    fn script_id(&self, object: u32) -> Result<i16, String> {
        CaptureView::script_id(&self.0, object)
    }
    fn capture(&mut self, object: u32) -> Result<snapshot::Position, String> {
        CaptureView::capture(&mut self.0, object)
    }
    fn definition(&self, object: u32) -> Result<u32, String> {
        CaptureView::definition(&self.0, object)
    }
    fn facing(&self, object: u32) -> Result<i16, String> {
        CaptureView::facing(&self.0, object)
    }
}
impl placement::View for CaptureOnly<'_> {
    fn facing(&self, object: u32) -> Result<i16, String> {
        CaptureView::facing(&self.0, object)
    }
}
impl lifecycle::View for CaptureOnly<'_> {
    fn manager(&self) -> Result<u32, String> {
        Err("capture/destruction check must not request a factory manager".into())
    }
}
