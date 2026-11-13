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
/// How a string table's entries are laid out, from its leading word.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// Version 0: length-prefixed strings.
    Pascal,
    /// Version -1: NUL-terminated strings.
    Terminated,
    /// Version -2: NUL-terminated strings, each followed by a comment.
    Commented,
    /// Version -3: a language code, then a string and a comment.
    Localized,
    /// A classic Mac OS `STR#` resource: no version word, a big-endian count,
    /// then length-prefixed strings. Base-game `Behavior.iff` and many
    /// downloads carry these. Any leading word that is not a known version
    /// is read this way.
    MacResource,
}
#[derive(Debug, Clone)]
pub struct StringTable {
    pub format: Format,
    pub entries: Vec<TextEntry>,
}
impl StringTable {
    pub fn parse(data: &[u8]) -> Result<Self> {
        let mut r = Reader::new(data);
        let word = r.u16()?;
        let format = match word as i16 {
            0 => Format::Pascal,
            -1 => Format::Terminated,
            -2 => Format::Commented,
            -3 => Format::Localized,
            v if v < 0 => return r.fail(format!("unsupported string-table version {v}")),
            _ => Format::MacResource,
        };
        let count = match format {
            Format::MacResource => usize::from(word.swap_bytes()),
            // A table holding only its version word is empty.
            _ if r.remaining() == 0 => 0,
            _ => usize::from(r.u16()?),
        };
        r.count(count, 1)?;
        let mut entries = Vec::with_capacity(count);
        for _ in 0..count {
            let language = match format {
                Format::Localized => r.u8()?.max(1),
                _ => 1,
            };
            let raw_text = match format {
                Format::Pascal | Format::MacResource => {
                    let n = usize::from(r.u8()?);
                    r.take(n)?
                }
                _ => r.cstr()?,
            };
            let raw_comment = match format {
                Format::Commented | Format::Localized => r.cstr()?,
                _ => &[],
            };
            entries.push(TextEntry {
                language,
                text: decode(raw_text, language),
                comment: decode(raw_comment, language),
                raw_text: raw_text.to_vec(),
                raw_comment: raw_comment.to_vec(),
            });
        }
        Ok(Self { format, entries })
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
mod tests;
