use crate::Result;
use crate::reader::Reader;

#[derive(Debug, Clone)]
pub struct TextEntry {
    pub language: u8,
    pub text: String,
    pub comment: String,
    pub raw_text: Vec<u8>,
    pub raw_comment: Vec<u8>,
}
#[derive(Debug, Clone)]
pub struct StringTable {
    pub version: i16,
    pub entries: Vec<TextEntry>,
}
impl StringTable {
    pub fn parse(data: &[u8]) -> Result<Self> {
        Self::read(data, true)
    }
    /// Check the complete table using the same byte reader and error ordering
    /// as `parse`, without retaining or decoding entries. Text decoding is
    /// infallible; it is needed only by callers consuming the text.
    pub fn validate(data: &[u8]) -> Result<()> {
        Self::read(data, false).map(|_| ())
    }
    fn read(data: &[u8], retain_entries: bool) -> Result<Self> {
        let mut r = Reader::new(data);
        let version = r.i16()?;
        if !matches!(version, -3..=0) {
            return r.fail(format!("unsupported string-table version {version}"));
        }
        if r.remaining() == 0 {
            return Ok(Self {
                version,
                entries: Vec::new(),
            });
        }
        let count = r.u16()? as usize;
        r.count(count, 1)?;
        let mut entries = if retain_entries {
            Vec::with_capacity(count)
        } else {
            Vec::new()
        };
        for _ in 0..count {
            let language = if version == -3 { r.u8()?.max(1) } else { 1 };
            let raw_text = if version == 0 {
                let n = r.u8()? as usize;
                r.take(n)?
            } else {
                r.cstr()?
            };
            let raw_comment = if version <= -2 { r.cstr()? } else { &[] };
            if retain_entries {
                entries.push(TextEntry {
                    language,
                    text: decode(raw_text, language),
                    comment: decode(raw_comment, language),
                    raw_text: raw_text.to_vec(),
                    raw_comment: raw_comment.to_vec(),
                });
            }
        }
        Ok(Self { version, entries })
    }
    pub fn get(&self, index: usize, language: u8) -> Option<&str> {
        self.entries
            .iter()
            .filter(|e| e.language == language)
            .nth(index)
            .or_else(|| self.entries.iter().filter(|e| e.language == 1).nth(index))
            .map(|e| e.text.as_str())
    }
}
/// Decode original game text bytes using the selected language's code page.
pub fn decode(bytes: &[u8], language: u8) -> String {
    use encoding_rs::*;
    let encoding = match language {
        12 => WINDOWS_1255,
        13 => WINDOWS_1251,
        15 => SHIFT_JIS,
        16 | 21 => WINDOWS_1250,
        17 => GBK,
        18 => BIG5,
        19 => WINDOWS_874,
        20 => EUC_KR,
        _ => WINDOWS_1252,
    };
    encoding.decode(bytes).0.into_owned()
}

#[cfg(test)]
#[path = "strings/validation_tests.rs"]
mod validation_tests;
