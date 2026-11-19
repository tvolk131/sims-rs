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
    /// downloads carry these. Any leading word that is not one of the four
    /// version words is read this way, including those that would be
    /// negative read as versions: 128 strings start `00 80`.
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
    /// Parses a resource's table as the original's string loader does
    /// (0x46feac), with `flags` the resource's flags
    /// ([`crate::iff::Iff::resource_flags`]):
    ///
    /// - Without flag 0x10 the resource is big-endian and 0x470351 swaps
    ///   its leading word, and, when that is then -1, the count.
    /// - A leading word of -1, -2 or -3 is then that version, with the
    ///   count after it.
    /// - Any other leading word is the count of length-prefixed strings
    ///   that follow it (0x4700d7); 0 or below is an empty table. (Version
    ///   0, which [`StringTable::parse`] reads with a count after it, is
    ///   not a layout the original knows.)
    ///
    /// **Not ported:** the original's change of bytes 0xa1 to `o` and 0xd5
    /// to `'` in length-prefixed strings (0x4700f5).
    pub fn parse_chunk(data: &[u8], flags: u16) -> Result<Self> {
        let Some(&[a, b]) = data.first_chunk::<2>() else {
            return Self::parse(data);
        };
        let swapped = flags & 0x10 == 0;
        let word = if swapped {
            i16::from_be_bytes([a, b])
        } else {
            i16::from_le_bytes([a, b])
        };
        let mut normal = data.to_vec();
        match word {
            -3..=-1 => {
                normal[..2].copy_from_slice(&word.to_le_bytes());
                if word == -1 && swapped && normal.len() >= 4 {
                    normal.swap(2, 3);
                }
            }
            count if count > 0 => {
                // `parse` reads a word that is not a version as a
                // big-endian count.
                normal[..2].copy_from_slice(&count.to_be_bytes());
            }
            _ => {
                return Ok(Self {
                    format: Format::MacResource,
                    entries: Vec::new(),
                });
            }
        }
        Self::parse(&normal)
    }
    pub fn get(&self, index: usize, language: u8) -> Option<&str> {
        self.entry(index, language).map(|e| e.text.as_str())
    }
    /// The entry [`StringTable::get`] reads its text from: the `index`th
    /// in `language`, else the `index`th in language 1.
    pub fn entry(&self, index: usize, language: u8) -> Option<&TextEntry> {
        self.entries
            .iter()
            .filter(|e| e.language == language)
            .nth(index)
            .or_else(|| self.entries.iter().filter(|e| e.language == 1).nth(index))
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
