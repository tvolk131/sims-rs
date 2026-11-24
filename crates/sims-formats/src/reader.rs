use std::fmt;

#[derive(Debug, Clone)]
pub struct Error {
    pub offset: usize,
    pub message: String,
}
impl Error {
    pub fn new(offset: usize, message: impl Into<String>) -> Self {
        Self {
            offset,
            message: message.into(),
        }
    }
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "at byte 0x{:X}: {}", self.offset, self.message)
    }
}
impl std::error::Error for Error {}
pub type Result<T> = std::result::Result<T, Error>;

pub(crate) struct Reader<'a> {
    pub data: &'a [u8],
    pub pos: usize,
}
impl<'a> Reader<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }
    pub fn remaining(&self) -> usize {
        self.data.len() - self.pos
    }
    pub fn fail<T>(&self, message: impl Into<String>) -> Result<T> {
        Err(Error::new(self.pos, message))
    }
    pub fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self
            .pos
            .checked_add(n)
            .ok_or_else(|| Error::new(self.pos, "size overflow"))?;
        let bytes = self.data.get(self.pos..end).ok_or_else(|| {
            Error::new(
                self.pos,
                format!("need {n} bytes; only {} remain", self.remaining()),
            )
        })?;
        self.pos = end;
        Ok(bytes)
    }
    pub fn skip(&mut self, n: usize) -> Result<()> {
        self.take(n).map(|_| ())
    }
    pub fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }
    pub fn u16(&mut self) -> Result<u16> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }
    pub fn i16(&mut self) -> Result<i16> {
        Ok(self.u16()? as i16)
    }
    pub fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    pub fn i32(&mut self) -> Result<i32> {
        Ok(self.u32()? as i32)
    }
    pub fn f32(&mut self) -> Result<f32> {
        Ok(f32::from_bits(self.u32()?))
    }
    pub fn be16(&mut self) -> Result<u16> {
        Ok(u16::from_be_bytes(self.take(2)?.try_into().unwrap()))
    }
    pub fn be32(&mut self) -> Result<u32> {
        Ok(u32::from_be_bytes(self.take(4)?.try_into().unwrap()))
    }
    pub fn cstr(&mut self) -> Result<&'a [u8]> {
        let n = self.data[self.pos..]
            .iter()
            .position(|b| *b == 0)
            .ok_or_else(|| Error::new(self.pos, "unterminated string"))?;
        let s = self.take(n)?;
        self.skip(1)?;
        Ok(s)
    }
    pub fn count(&self, count: usize, minimum: usize) -> Result<()> {
        if count > self.remaining() / minimum {
            self.fail(format!("count {count} exceeds remaining data"))
        } else {
            Ok(())
        }
    }
}
/// Text the original holds, read in the Windows-1252 code page. Every byte
/// is read as itself: a leading byte-order mark is three characters of the
/// text, as the original's byte comparisons see it, not a sign of another
/// encoding.
pub(crate) fn western(bytes: &[u8]) -> String {
    encoding_rs::WINDOWS_1252
        .decode_without_bom_handling(bytes)
        .0
        .into_owned()
}

/// Encodes text as Windows-1252, the inverse of [`western`]. `None` when a
/// character has no Windows-1252 byte or is NUL, which would end the C
/// string early.
pub(crate) fn encode_western(text: &str) -> Option<Vec<u8>> {
    let (bytes, _, unmappable) = encoding_rs::WINDOWS_1252.encode(text);
    (!unmappable && !bytes.contains(&0)).then(|| bytes.into_owned())
}

/// The little-endian stream the original's raw (unpacked) mode writes.
#[derive(Debug, Default)]
pub(crate) struct Out(pub(crate) Vec<u8>);

impl Out {
    /// The saved-resource header the stream writer puts first (0x464d40):
    /// four zero bytes, the version, and the tag as a little-endian word.
    pub(crate) fn header(version: u32, magic: &[u8; 4]) -> Self {
        let mut out = Self(vec![0; 4]);
        out.u32(version);
        out.0.extend(magic);
        out
    }
    pub(crate) fn u8(&mut self, value: u8) {
        self.0.push(value);
    }
    pub(crate) fn i16(&mut self, value: i16) {
        self.0.extend(value.to_le_bytes());
    }
    pub(crate) fn u16(&mut self, value: u16) {
        self.0.extend(value.to_le_bytes());
    }
    pub(crate) fn i32(&mut self, value: i32) {
        self.0.extend(value.to_le_bytes());
    }
    pub(crate) fn u32(&mut self, value: u32) {
        self.0.extend(value.to_le_bytes());
    }
    pub(crate) fn len32(&mut self, len: usize) -> crate::Result<()> {
        let len = u32::try_from(len)
            .map_err(|_| crate::Error::new(self.0.len(), "count exceeds 32 bits"))?;
        self.u32(len);
        Ok(())
    }
    /// A C string padded to an even stream offset, as [`crate::lot`]'s
    /// `padded_string` reads it. The original's string writer only moves
    /// past the padding byte (0x464cd7), so the caller supplies it.
    pub(crate) fn padded_string(
        &mut self,
        text: &str,
        pad: impl FnOnce() -> u8,
    ) -> crate::Result<()> {
        let bytes = encode_western(text)
            .ok_or_else(|| crate::Error::new(self.0.len(), "string is not Windows-1252 text"))?;
        self.0.extend(bytes);
        self.0.push(0);
        if self.0.len() & 1 != 0 {
            self.0.push(pad());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn western_text_encodes_back_to_its_bytes() {
        let bytes: Vec<u8> = (1..=255).collect();
        let text = super::western(&bytes);
        assert_eq!(super::encode_western(&text).unwrap(), bytes);
        assert_eq!(super::encode_western("a\0b"), None);
        assert_eq!(super::encode_western("\u{4e00}"), None);
    }

    #[test]
    fn a_byte_order_mark_is_read_as_three_characters() {
        assert_eq!(
            super::western(b"\xef\xbb\xbfplain.flr"),
            "\u{ef}\u{bb}\u{bf}plain.flr"
        );
        assert_eq!(super::western(b"\xff\xfeA"), "\u{ff}\u{fe}A");
        assert_eq!(super::western(b"plain.flr"), "plain.flr");
    }
}
