//! Stored CPState availability and visibility of the funds/secondary widgets.
use super::*;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Access {
    sources: PanelAccess,
    funds_visible: bool,
    label_visible: bool,
    /// Populated by the separate 53a01f update, not by ViewControl's visibility
    /// call. A newly attached replacement widget has no published text yet.
    secondary_text: String,
}
impl Access {
    pub(super) fn new(sources: PanelAccess) -> Self {
        Self {
            sources,
            funds_visible: true,
            label_visible: true,
            secondary_text: String::new(),
        }
    }
    pub fn funds_visible(&self) -> bool {
        self.funds_visible
    }
    pub fn label_visible(&self) -> bool {
        self.label_visible
    }
    pub fn secondary_text(&self) -> &str {
        &self.secondary_text
    }
    pub(super) fn refresh_fire(&mut self, blocked: bool) {
        self.sources.buy_blocked = blocked;
    }
    pub(super) fn service(
        &mut self,
        owners: &Owners,
        r: control::Request,
    ) -> Result<Option<control::Reply>> {
        match r {
            control::Request::Get { owner, query } => {
                let value = match query {
                    control::Query::BuyBlocked => self.sources.buy_blocked,
                    control::Query::LiveBlocked => self.sources.live_blocked,
                    control::Query::LabelsHidden => self.sources.labels_hidden,
                    _ => return Ok(None),
                };
                ensure!(owner == owners.state, "panel availability owner differs");
                Ok(Some(control::Reply::Number(u32::from(value))))
            }
            control::Request::Visibility { widget, slot } => {
                let visible = match slot {
                    0x94 => true,
                    0x98 => false,
                    _ => anyhow::bail!("unsupported money widget visibility"),
                };
                if widget == owners.widgets[control::Widget::Status as usize] {
                    self.funds_visible = visible;
                } else if widget == owners.widgets[control::Widget::Label as usize] {
                    self.label_visible = visible;
                } else {
                    return Ok(None);
                }
                Ok(Some(control::Reply::Done))
            }
            _ => Ok(None),
        }
    }
}
