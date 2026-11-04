//! 5c17a3's filename key and 5c1774's resource-list duplicate predicate.
//! The removed extension remains in counted storage beyond an embedded NUL.
use super::generator::fixed;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Name {
    pub identity: u32,
    pub bytes: Vec<u8>,
}
impl Name {
    pub fn parse(filename: &[u8]) -> Result<Self, String> {
        let text = fixed(filename, usize::MAX);
        let (identity, start) = if text.first() == Some(&b'_') {
            let end = text[1..]
                .iter()
                .position(|&b| b == b'_')
                .ok_or("native resource ID prefix is unterminated")?
                + 1;
            if end - 1 > 11 {
                return Err("native resource ID prefix exceeds temporary buffer".into());
            }
            (
                crate::pose::layer::atoi(&String::from_utf8_lossy(&text[1..end])) as u32,
                end + 1,
            )
        } else {
            (0x80000000, 0)
        };
        let mut bytes = text[start..].to_vec();
        // The original walks backward. Lowercasing is byte-based in the
        // captured C locale; no Unicode/code-page case expansion is performed.
        bytes.make_ascii_lowercase();
        if let Some(dot) = bytes.iter().rposition(|&b| b == b'.') {
            bytes[dot] = 0;
        }
        Ok(Self { identity, bytes })
    }

    pub fn visible(&self) -> &[u8] {
        fixed(&self.bytes, usize::MAX)
    }
}
