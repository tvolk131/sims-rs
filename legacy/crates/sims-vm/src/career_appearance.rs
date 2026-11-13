//! Person-owned 45ee5f cache (+b2c..+bd0), independent of animator reloads.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    pub key: [i16; 2],
    pub body: Vec<u8>,
    pub texture: Vec<u8>,
    pub extra: Vec<u8>,
}
impl Default for State {
    fn default() -> Self {
        // 459467/45947b, not the animator's -1 selector cache.
        Self {
            key: [-999, -1],
            body: vec![],
            texture: vec![],
            extra: vec![],
        }
    }
}
impl State {
    pub fn validate(&self) -> Result<(), String> {
        if [&self.body, &self.texture, &self.extra]
            .into_iter()
            .any(|s| s.len() > 63 || s.contains(&0))
        {
            return Err("invalid saved career appearance fixed string".into());
        }
        Ok(())
    }
}
