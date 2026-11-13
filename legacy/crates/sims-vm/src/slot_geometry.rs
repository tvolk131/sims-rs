//! Current contained-slot geometry, independent of its SLOT initialization file.
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Anchor {
    Absent,
    Named(Vec<u8>),
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Slot {
    pub offset: [u32; 3],
    pub height: i32,
    /// None is unobserved; a native null pointer is explicitly Absent.
    #[serde(default)]
    pub anchor: Option<Anchor>,
}
impl Slot {
    pub fn validate(&self) -> Result<(), String> {
        if self.offset.iter().any(|&v| !f32::from_bits(v).is_finite()) {
            return Err("nonfinite retained containment slot offset".into());
        }
        if let Some(Anchor::Named(name)) = &self.anchor {
            if name.len() > 65535 || name.contains(&0) {
                return Err("invalid native slot anchor name".into());
            }
        }
        Ok(())
    }
}
